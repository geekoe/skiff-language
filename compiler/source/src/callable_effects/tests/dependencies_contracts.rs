use super::support::*;

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
                direct_return_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
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

    assert_eq!(
        effects(&model, "wrapper"),
        CallableMayEffects {
            may_suspend: true,
            ..dependency_effects
        }
    );
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
                exact_signature,
                ..
            } if package_requirement_alias == "dep"
                && package_callable_id == &PackageCallableId::new("pkg-callable:dep-run")
                && expected_local_abi == &PackageLocalAbiIdentity::new("pkg-local-abi:dep")
                && exact_signature.is_none()
        )
    }));
}

#[test]
fn dependency_exact_signature_controls_caller_suspension() {
    let callable = |id: &str, may_suspend| {
        PackageDependencyCallableAnalysis::new(
            PackageCallableId::new(id),
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
                name: "input".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
            may_suspend,
        })
    };
    let dependencies = SourceDependencyAnalysisInput::new(
        BTreeMap::from([(
            "dep".to_string(),
            PackageDependencyAnalysisFacts::new(
                skiff_artifact_model::PackageBuildId::new("build:dep"),
                PackageLocalAbiIdentity::new("pkg-local-abi:dep"),
                BTreeMap::from([
                    (
                        "exactFalse".to_string(),
                        callable("pkg-callable:dep-exact-false", false),
                    ),
                    (
                        "exactTrue".to_string(),
                        callable("pkg-callable:dep-exact-true", true),
                    ),
                ]),
            ),
        )]),
        Vec::new(),
    )
    .unwrap();
    let model = analyze_with_dependency_artifact(
        r#"
            function exactFalse(input: string) -> string {
              return dep/exactFalse(input)
            }

            function exactTrue(input: string) -> string {
              return dep/exactTrue(input)
            }
        "#,
        dependencies,
    );

    assert_eq!(effects(&model, "exactFalse"), no_effects());
    assert_eq!(effects(&model, "exactTrue"), suspend_only_effects());
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

    assert_detached_contract_summary(&model, "wrapper");
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

    assert_detached_contract_summary(&model, "wrapper");
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
fn missing_detached_error_or_other_guarantee_remains_fail_closed() {
    for missing_guarantee in ["detached_error", "no_caller_value_escape"] {
        let (mut contract, schema) =
            contract_and_schema("example.echo", "1.0.0", "send", "payload", "payloadClosure");
        let guarantee = &mut contract
            .operations
            .values_mut()
            .next()
            .unwrap()
            .contract
            .effect_guarantee;
        match missing_guarantee {
            "detached_error" => guarantee.detached_error = false,
            "no_caller_value_escape" => guarantee.no_caller_value_escape = false,
            _ => unreachable!(),
        }
        assign_service_contract_identities(&mut contract).unwrap();
        let dependency = ResolvedContractDependency::validated(
            requirement("echo", &contract),
            contract,
            &[schema],
        )
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
        assert!(effects.writes_caller_reachable, "{missing_guarantee}");
        assert!(effects.throws_caller_alias, "{missing_guarantee}");
        assert!(effects.escapes_caller_value, "{missing_guarantee}");
        assert!(
            !effects.requires_same_heap_identity,
            "{missing_guarantee}: fail-closed must not invent identity observation"
        );
        assert!(effects.invokes_unknown_target, "{missing_guarantee}");
        assert!(effects.may_suspend, "{missing_guarantee}");
        assert!(
            matches!(
                provenance(&model, "wrapper"),
                CallableProvenanceSummary::Unknown {
                    reason: CallableProvenanceUnknownReason::UnknownCallTarget
                }
            ),
            "{missing_guarantee}"
        );
    }
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
