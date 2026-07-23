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
    contract_dependency_test_fixture::{contract_fixture, requirement, resolved_contract_fixture},
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
                module_path,
                function_name
            } if module_path == "api" && function_name == "detach"
        )
    }));
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
        "httpWrapper",
        "dynamicNativeWrapper",
        "dynamicWrapper",
        "interfaceWrapper",
        "mutableReceiver",
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
                module_path,
                function_name,
            } if module_path == "std.effect_test" && function_name == "customNative"
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
    let mut contract = contract_fixture(
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
    assign_service_contract_identities(&mut contract).unwrap();
    let dependency =
        ResolvedContractDependency::validated(requirement("echo", &contract), contract).unwrap();
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
    let mut contract =
        contract_fixture("example.echo", "1.0.0", "send", "payload", "payloadClosure");
    let operation = contract.operations.values_mut().next().unwrap();
    operation.contract.return_value.ty = ContractTypeRef::builtin("string");
    operation.contract.may_suspend = true;
    assign_service_contract_identities(&mut contract).unwrap();
    let dependency =
        ResolvedContractDependency::validated(requirement("echo", &contract), contract).unwrap();
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
    let mut contract =
        contract_fixture("example.echo", "1.0.0", "send", "payload", "payloadClosure");
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
        ResolvedContractDependency::validated(requirement("echo", &contract), contract).unwrap();
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
            platform_package_authority: None,
        },
        &dependency_analysis,
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
