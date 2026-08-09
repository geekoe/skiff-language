use super::*;

#[test]
fn generic_callable_binders_survive_executable_and_public_signature_handoff() {
    let model = build_model(
        r#"
            function submit<T, Id>(id: Id) -> T? {
                return null
            }
        "#,
        &SourceDependencyAnalysisInput::default(),
        &BTreeMap::new(),
    )
    .expect("generic callable signature builds");

    let executable = model
        .executable_signatures()
        .signature(&crate::SourceSymbolKey::new("api", "submit"))
        .expect("generic executable signature");
    assert_eq!(executable.type_params, ["T", "Id"]);
    assert!(matches!(
        &executable.parameters[0].ty,
        PackageTypeRef::Local {
            local_type: TypeRefIr::TypeParam { name }
        } if name == "Id"
    ));

    let public = model
        .callable_signatures()
        .signature("submit")
        .expect("generic public signature");
    assert_eq!(public.type_params, ["T", "Id"]);
    assert!(matches!(
        &public.return_type,
        PackageTypeRef::Nullable { inner }
            if matches!(
                inner.as_ref(),
                PackageTypeRef::Local {
                    local_type: TypeRefIr::TypeParam { name }
                } if name == "T"
            )
    ));
}

#[test]
fn explicit_receiver_is_owned_by_the_executable_fact_and_trimmed_once_from_public_view() {
    let dependency_analysis = contract_dependencies();
    let publication_api = PublicationApiSpec::from_public_instances(vec![
        PublicationApiPublicInstanceEntry::for_source(
            "handler",
            "root.api.handler",
            ["root.api.PublicApi"],
        )
        .unwrap(),
    ]);
    let model = build_model_with_publication_api(
        r#"
            interface PublicApi {
              function submit(self: Self, input: payments.User) -> payments.User
            }
            type Handler implements PublicApi {}
            impl Handler {
              function submit(self: Handler, input: payments.User) -> payments.User {
                return input
              }
            }
            const handler: Handler = Handler {}
        "#,
        &dependency_analysis,
        &BTreeMap::new(),
        &[],
        &publication_api,
    )
    .expect("explicit receiver signature builds");

    let executable = model
        .executable_signatures()
        .signature(&crate::SourceSymbolKey::new("api", "Handler.submit"))
        .expect("method executable fact");
    assert!(matches!(
        executable.receiver,
        SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 }
    ));
    assert_eq!(executable.parameters[0].name, "self");
    assert_eq!(executable.parameters[1].name, "input");

    let public = model
        .callable_signatures()
        .signature("handler.submit")
        .expect("public callable view");
    assert_eq!(
        public
            .parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["input"]
    );
}

#[test]
fn public_view_fails_when_its_canonical_executable_fact_is_missing() {
    let dependency_analysis = contract_dependencies();
    let model = build_model(
        "function submit(input: payments.User) -> void {}",
        &dependency_analysis,
        &BTreeMap::new(),
    )
    .expect("fixture model builds");
    let mut executable_signatures = model.executable_signatures().clone();
    executable_signatures
        .by_source_key
        .remove(&crate::SourceSymbolKey::new("api", "submit"));

    let error = SourceCallableSignatureFacts::build(
        model.sources().parsed_sources(),
        model.export_bindings(),
        model.type_resolution(),
        &executable_signatures,
    )
    .expect_err("public view cannot reconstruct a missing executable fact");
    assert!(error.contains("has no exact source executable signature fact"));
}

#[test]
fn duplicate_source_executable_fact_fails_closed() {
    let dependency_analysis = contract_dependencies();
    let model = build_model(
        "function submit(input: payments.User) -> void {}",
        &dependency_analysis,
        &BTreeMap::new(),
    )
    .expect("fixture model builds");
    let source = model.sources().parsed_sources()[0].clone();
    let duplicate_sources = vec![source.clone(), source];

    let error = SourceExecutableSignatureFacts::build(
        &duplicate_sources,
        model.type_resolution(),
        &dependency_analysis,
        model.callable_effects(),
    )
    .expect_err("a source key cannot receive two exact signature facts");
    assert!(error.contains("has more than one exact signature fact"));
}

#[test]
fn inline_source_shape_preserves_nested_contract_nominal_identity() {
    let dependency_analysis = contract_dependencies();
    let model = build_model(
        "function submit(input: { user: payments.User }) -> void {}",
        &dependency_analysis,
        &BTreeMap::new(),
    )
    .expect("inline source shape should retain the exact nested contract nominal");
    let signature = model
        .executable_signatures()
        .signature(&crate::SourceSymbolKey::new("api", "submit"))
        .expect("submit executable signature");
    let PackageTypeRef::Local {
        local_type: TypeRefIr::Record { fields },
    } = &signature.parameters[0].ty
    else {
        panic!("inline record should remain a structural local type")
    };
    assert!(matches!(
        fields.get("user"),
        Some(TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        }) if package_id == "example.payments.package" && stable_schema_key == "User"
    ));
}

#[test]
fn missing_and_pending_effect_facts_cannot_seed_an_executable_signature() {
    let dependency_analysis = contract_dependencies();
    let (parsed_sources, type_resolution) = parsed_type_model(
        "function submit(input: payments.User) -> void {}",
        "missing-executable-effects",
    );

    let missing = SourceExecutableSignatureFacts::build(
        &parsed_sources,
        &type_resolution,
        &dependency_analysis,
        &crate::SourceCallableEffectFacts::default(),
    )
    .expect_err("missing effect fact must fail exact signature construction");
    assert!(missing.contains("has no source-owned effect fact"));

    let pending_effects = crate::SourceCallableEffectFacts::analysis_pending(&parsed_sources);
    let pending = SourceExecutableSignatureFacts::build(
        &parsed_sources,
        &type_resolution,
        &dependency_analysis,
        &pending_effects,
    )
    .expect_err("analysis-pending effect cannot become an exact may_suspend fact");
    assert!(pending.contains("has unknown source effects: AnalysisPending"));
}

fn parsed_type_model(
    source_text: &str,
    fixture_name: &str,
) -> (
    Vec<crate::parsed_sources::ParsedCompilerSource>,
    TypeResolutionModel,
) {
    let source = CompilerSourceFile::parse(
        PathBuf::from("api.skiff"),
        "api".to_string(),
        true,
        false,
        source_text.to_string(),
        "api.skiff",
    )
    .expect("fixture parses");
    let fixture_root = PathBuf::from(format!("/tmp/{fixture_name}"));
    let parsed_sources =
        parse_publication_sources(&fixture_root, &[source]).expect("fixture source facts build");
    let type_symbols = crate::publication_type_symbols(&parsed_sources);
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::new(),
        &[],
        None,
        None,
        &type_symbols,
    )
    .expect("ordinary source types resolve before exact projection");
    (parsed_sources, type_resolution)
}
