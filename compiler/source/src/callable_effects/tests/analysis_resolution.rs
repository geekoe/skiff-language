use super::support::*;

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

    let production = AnalysisFixture::new(source_text).analyze();
    assert_eq!(effects(&production, "run"), no_effects());
}

#[test]
fn simple_detached_wrapper_is_safe_and_direct_transitive_calls_resolve() {
    let model = AnalysisFixture::new(
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
    )
    .analyze();

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
                source_callable,
                ..
            } if source_callable == &SourceSymbolKey::new("api", "detach")
        )
    }));
}

#[test]
fn nested_local_calls_preserve_exact_effects_and_provenance() {
    let model = AnalysisFixture::new(
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
              final rows = db find many Input {
                where value == input.value
              }
              return outer(inner(input))
            }

            function nestedRecordField(input: Input) -> Output {
              final rows = db find many Input {
                where value == input.value
              }
              return Output { value: inner(input).value }
            }

            function nestedCollectionElement(input: Input) -> JsonObject {
              final rows = db find many Input {
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
    )
    .analyze();

    for callable in ["nested", "nestedRecordField", "nestedCollectionElement"] {
        assert_eq!(
            effects(&model, callable),
            pending_only_effects(vec![PendingEffectCategory::HostEffect]),
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
                ResolvedCallTarget::LocalFunction {
                    source_callable,
                    ..
                }
                    if source_callable == &SourceSymbolKey::new("api", "inner")
            ))
            .count()
            >= 4,
        "distinct inner call sites must keep distinct expression keys"
    );
}

#[test]
fn module_constant_return_keeps_exact_constant_provenance_through_local_call() {
    let model = AnalysisFixture::sources(&[
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
    ])
    .analyze();

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
    let model = AnalysisFixture::new(
        r#"
            function compute() -> string { return "computed" }

            const UNSUPPORTED: string = compute()
            const CYCLE_A: string = CYCLE_B
            const CYCLE_B: string = CYCLE_A

            function unsupportedValue() -> string { return UNSUPPORTED }
            function cyclicValue() -> string { return CYCLE_A }
        "#,
    )
    .analyze();

    for callable in ["unsupportedValue", "cyclicValue"] {
        assert_eq!(effects(&model, callable), all_effects(), "{callable}");
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
    let unresolved =
        AnalysisFixture::new("function unresolved() -> string { return MISSING_GLOBAL }")
            .analyze_result()
            .expect("source analysis retains a fail-closed callable summary");
    assert_eq!(effects(&unresolved, "unresolved"), all_effects());
    assert_eq!(
        provenance(&unresolved, "unresolved"),
        &CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnknownCallTarget,
        }
    );

    let model = AnalysisFixture::new(
        r#"
            type Boxed { value: string }
            function freshValue() -> Boxed { return Boxed { value: "fresh" } }
            function wrapper() -> Boxed { return freshValue() }
        "#,
    )
    .analyze();
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
    let model = AnalysisFixture::new(
        r#"
            type Input { value: string }
            type Output { value: string }
            type Failure = string

            function detach(input: Input) -> Output {
              return Output { value: input.value }
            }

            function rootWrapper(input: Input) -> Output {
              return root.api.detach(input)
            }

            function catchWrapper(input: Input) -> Output? {
              final attempted = catch<Failure>(detach(input))
              if attempted.tag == "ok" { return attempted.value }
              return null
            }
        "#,
    )
    .analyze();

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
    let model = AnalysisFixture::new(
        r#"
            type Boxed { value: string }
            type Failure = string

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
              final attempted = catch<Failure>(fresh(input))
              if attempted.tag == "ok" { return attempted.value }
              return null
            }

            function okNeEarly(input: Boxed) -> Boxed? {
              final attempted = catch<Failure>(fresh(input))
              if attempted.tag != "ok" { return null }
              return attempted.value
            }

            function nested(input: Boxed) -> Boxed? {
              final attempted = catch<Failure>(okEq(input))
              if attempted.tag != "ok" { return null }
              return attempted.value
            }

            function exactAlias(input: Boxed) -> Boxed? {
              final attempted = catch<Failure>(alias(input))
              if attempted.tag != "ok" { return null }
              return attempted.value
            }

            function errorBranch(input: Boxed) -> Exception<Failure>? {
              final attempted = catch<Failure>(alias(input))
              if attempted.tag == "err" { return attempted.exception }
              return null
            }

            function nullableCheck(input: Boxed) -> bool {
              final attempted = catch<Failure>(nullableAlias(input))
              if attempted.tag != "ok" { return false }
              return attempted.value == null
            }
        "#,
    )
    .analyze();

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

    assert_eq!(effects(&model, "exactAlias"), no_effects());
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
    let model = AnalysisFixture::new(
        r#"
            type Boxed { value: string }
            type Failure = string

            interface Provider {
              function run(self: Self, input: Boxed) -> Boxed
            }

            function unknown(input: Boxed, provider: any Provider) -> Boxed? {
              final attempted = catch<Failure>(provider.run(input))
              if attempted.tag != "ok" { return null }
              return attempted.value
            }
        "#,
    )
    .analyze();

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
    let model = AnalysisFixture::sources(&[
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
    ])
    .analyze();

    assert_eq!(effects_in(&model, "relay", "handler"), no_effects());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::LocalFunction {
                source_callable,
                ..
            } if source_callable == &SourceSymbolKey::new("helpers", "detach")
        )
    }));
}

#[test]
fn publication_wide_call_graph_closes_effects_and_provenance_across_files() {
    let model = AnalysisFixture::sources(&[
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
    ])
    .analyze();

    for (module, symbol) in [
        ("effects", "returnPayload"),
        ("bridge", "returnPayload"),
        ("entry", "returnThroughFiles"),
        ("bridge", "recursive"),
        ("entry", "recursiveThroughFiles"),
    ] {
        assert_eq!(
            effects_in(&model, module, symbol),
            no_effects(),
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
        assert_eq!(effects, no_effects(), "{module}.{symbol}");
        assert!(!effects.requires_same_heap_identity, "{module}.{symbol}");
    }

    for (module, symbol) in [
        ("effects", "persist"),
        ("bridge", "persist"),
        ("entry", "persistThroughFiles"),
    ] {
        let effects = effects_in(&model, module, symbol);
        assert!(effects.escapes_caller_value, "{module}.{symbol}");
        assert_eq!(
            effects,
            CallableMayEffects {
                escapes_caller_value: true,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::HostEffect],
                ..no_effects()
            },
            "{module}.{symbol}"
        );
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

    let ambiguous = AnalysisFixture::sources(&[
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
    .analyze_result()
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
    let model = AnalysisFixture::new(
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
    )
    .analyze();

    assert_eq!(effects(&model, "wrapper"), no_effects());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::LocalImplMethod {
                source_callable,
                ..
            } if source_callable == &SourceSymbolKey::new("api", "ExactProvider.read")
        )
    }));
}

#[test]
fn generic_local_receiver_call_target_carries_exact_receiver_instantiation() {
    let model = AnalysisFixture::new(
        r#"
            type Box<T> { value: T }

            impl Box<T> {
              function unwrap() -> T {
                return self.value
              }
            }

            function wrapper(box: Box<string>) -> string {
              return box.unwrap()
            }
        "#,
    )
    .analyze();

    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::LocalImplMethod {
                source_callable,
                receiver_type_arguments,
                ..
            } if source_callable == &SourceSymbolKey::new("api", "Box<T>.unwrap")
                && receiver_type_arguments == &vec![TypeRefIr::builtin("string")]
        )
    }));
}

#[test]
fn interface_conformance_accepts_non_suspending_and_suspending_implementations() {
    let model = AnalysisFixture::new(
        r#"
            interface Runner {
              function run(self: Self) -> void
            }

            type Immediate implements Runner {}
            type Deferred implements Runner {}

            impl Immediate {
              function run() -> void {}
            }

            impl Deferred {
              function run() -> void {
                std.time.sleep(Duration.milliseconds(1))
              }
            }
        "#,
    )
    .analyze();

    assert_eq!(model.interface_signatures().conformances().count(), 2);
    assert_eq!(effects(&model, "Immediate.run"), no_effects());
    assert_eq!(
        effects(&model, "Deferred.run"),
        pending_only_effects(vec![PendingEffectCategory::NativeCall])
    );
}

#[test]
fn actor_receiver_call_uses_actor_method_target_and_exact_local_effects() {
    let model = AnalysisFixture::new(
        r#"
            type Worker {
              id: string,
              label: string,
            }

            actor Worker {
              key(id)
              create(label: string)
            }

            impl Worker {
              function create(self: Worker, label: string) -> void {
                self.label = label
              }

              function handle(self: Worker, value: string) -> string {
                return value
              }
            }

            function wrapper(worker: Worker, value: string) -> string {
              return worker.handle(value)
            }
        "#,
    )
    .analyze();

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
    let model = AnalysisFixture::new(
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
    )
    .analyze();

    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::LocalImplMethod {
                source_callable,
                ..
            } if source_callable == &SourceSymbolKey::new("api", "Worker.handle")
        )
    }));
    assert!(!model
        .resolved_call_targets()
        .iter()
        .any(|(_, target)| { matches!(target, ResolvedCallTarget::ActorMethod { .. }) }));
}

#[test]
fn missing_dynamic_mutable_and_capability_semantics_remain_fail_closed() {
    let model = AnalysisFixture::new(
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
              final callable = std.string.encodePath
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
    )
    .module("std.effect_test")
    .package(crate::shared::id::SKIFF_STD_PUBLICATION_ID)
    .analyze();

    for callable in [
        "customNative",
        "nativeWrapper",
        "fileWrapper",
        "dynamicNativeWrapper",
        "interfaceWrapper",
    ] {
        let effects = effects_in(&model, "std.effect_test", callable);
        assert!(effects.invokes_unknown_target, "{callable}");
        assert!(!effects.requires_same_heap_identity, "{callable}");
        assert!(matches!(
            provenance_in(&model, "std.effect_test", callable),
            CallableProvenanceSummary::Unknown { .. }
        ));
    }
    assert_eq!(
        effects_in(&model, "std.effect_test", "mutableReceiver"),
        no_effects()
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
        pending_only_effects(vec![PendingEffectCategory::HostEffect])
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
                    ..
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
            ResolvedCallTarget::InterfaceMethod {
                interface,
                method_abi_id,
                slot: 0,
            } if !interface.interface_abi_id.is_empty() && !method_abi_id.is_empty()
        )
    }));
}
