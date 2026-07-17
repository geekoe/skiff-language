use std::{collections::BTreeMap, path::Path, path::PathBuf};

use skiff_artifact_model::{
    CallableEffectSummary, CallableEffectUnknownReason, CallableMayEffects,
    CallableProvenanceSummary, CallableSemanticFacts, ContractOperationId, PackageCallableId,
    PackageLocalAbiIdentity, ServiceProtocolIdentity, ValueEscapeLane, ValueProvenance,
};

use crate::{
    build_from_parsed_sources_with_dependency_analysis, parsed_sources::parse_publication_sources,
    source_graph::CompilerSourceFile, CompileParsedPublicationSourcesInput,
    ContractDependencyAnalysisFacts, PackageDependencyAnalysisFacts,
    PackageDependencyCallableAnalysis, PublicationCompilePolicy, ResolvedCallTarget,
    SourceCompileModel, SourceDependencyAnalysisInput, SourceSymbolKey,
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
fn direct_and_transitive_parameter_write_propagate() {
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

    assert!(effects(&model, "mutate").writes_caller_reachable);
    assert!(effects(&model, "wrapper").writes_caller_reachable);
    assert!(effects(&model, "Boxed.clear").writes_caller_reachable);
    assert!(effects(&model, "methodWrapper").writes_caller_reachable);
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
fn native_and_unresolved_dynamic_targets_fail_closed() {
    let model = analyze_named(
        r#"
            type Boxed { value: string }
            interface Provider {
              function name(self: Self) -> string
            }
            native function host(input: Boxed) -> Boxed

            function nativeWrapper(input: Boxed) -> Boxed {
              return host(input)
            }

            function dynamicWrapper(input: Boxed) -> string {
              return input.value.concat("!")
            }

            function interfaceWrapper(input: any Provider) -> string {
              return input.name()
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "std.effect_test",
        crate::shared::id::SKIFF_STD_PUBLICATION_ID,
    );

    for callable in ["nativeWrapper", "dynamicWrapper", "interfaceWrapper"] {
        let effects = effects_in(&model, "std.effect_test", callable);
        assert!(effects.invokes_unknown_target, "{callable}");
        assert!(effects.requires_same_heap_identity, "{callable}");
        assert!(matches!(
            provenance_in(&model, "std.effect_test", callable),
            CallableProvenanceSummary::Unknown { .. }
        ));
    }
    assert_eq!(
        effects_in(&model, "std.effect_test", "nativeWrapper"),
        all_effects()
    );
    assert_eq!(
        effects_in(&model, "std.effect_test", "interfaceWrapper"),
        all_effects()
    );
}

#[test]
fn canonical_package_facts_import_effects_and_stable_target_identity() {
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
                BTreeMap::from([("run".to_string(), dependency)]),
            ),
        )]),
        BTreeMap::new(),
    );
    let model = analyze(
        r#"
            type Boxed { value: string }
            function wrapper(input: Boxed) -> Boxed {
              return dep.run(input)
            }
        "#,
        dependency_input,
    );

    let wrapper = effects(&model, "wrapper");
    assert!(wrapper.writes_caller_reachable);
    assert!(wrapper.returns_caller_alias);
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
fn contract_target_is_typed_but_effects_fail_closed_without_descriptor_facts() {
    let dependency_input = SourceDependencyAnalysisInput::new(
        BTreeMap::new(),
        BTreeMap::from([(
            "echo".to_string(),
            ContractDependencyAnalysisFacts::new(
                ServiceProtocolIdentity::new("service-protocol:echo"),
                BTreeMap::from([(
                    "send".to_string(),
                    ContractOperationId::new("contract-operation:send"),
                )]),
            ),
        )]),
    );
    let model = analyze(
        r#"
            function wrapper(input: string) -> void {
              echo.send(input)
            }
        "#,
        dependency_input,
    );

    assert!(effects(&model, "wrapper").invokes_unknown_target);
    assert!(effects(&model, "wrapper").escapes_caller_value);
    assert!(matches!(
        provenance(&model, "wrapper"),
        CallableProvenanceSummary::Unknown { .. }
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ContractOperation {
                contract_requirement_alias,
                contract_operation_id,
                expected_protocol_identity,
            } if contract_requirement_alias == "echo"
                && contract_operation_id == &ContractOperationId::new("contract-operation:send")
                && expected_protocol_identity
                    == &ServiceProtocolIdentity::new("service-protocol:echo")
        )
    }));
}

fn analyze(source: &str, dependency_analysis: SourceDependencyAnalysisInput) -> SourceCompileModel {
    analyze_named(
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
) -> SourceCompileModel {
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
    build_from_parsed_sources_with_dependency_analysis(
        CompileParsedPublicationSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: Path::new("/tmp/effect-provenance"),
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            service_dependencies: Default::default(),
            service_ingress: None,
            policy: PublicationCompilePolicy::Package { package_id },
        },
        &dependency_analysis,
    )
    .expect("source model builds")
}

fn effects(model: &SourceCompileModel, symbol: &str) -> CallableMayEffects {
    effects_in(model, "api", symbol)
}

fn effects_in(model: &SourceCompileModel, module: &str, symbol: &str) -> CallableMayEffects {
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

fn provenance<'a>(model: &'a SourceCompileModel, symbol: &str) -> &'a CallableProvenanceSummary {
    provenance_in(model, "api", symbol)
}

fn provenance_in<'a>(
    model: &'a SourceCompileModel,
    module: &str,
    symbol: &str,
) -> &'a CallableProvenanceSummary {
    model
        .callable_provenance()
        .operations()
        .get(&SourceSymbolKey::new(module, symbol))
        .unwrap_or_else(|| panic!("missing provenance for {symbol}"))
}

fn assert_escape_lane(model: &SourceCompileModel, symbol: &str, expected: ValueEscapeLane) {
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
