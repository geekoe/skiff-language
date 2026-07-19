use super::*;

fn contract_dependencies_with(aliases: &[(&str, &str)]) -> SourceDependencyAnalysisInput {
    SourceDependencyAnalysisInput::new(
        Vec::new(),
        aliases.iter().map(|(alias, service_id)| {
            resolved_contract_fixture(alias, service_id, "submit", "User", "Secret")
        }),
    )
    .unwrap()
}

fn build_interface_model(
    source: &str,
    dependencies: &SourceDependencyAnalysisInput,
) -> Result<PackageSourceModel, SourceCompileError> {
    build_model_with_publication_api(
        source,
        dependencies,
        &BTreeMap::new(),
        &[],
        &PublicationApiSpec::empty(),
    )
}

fn repository_source(implementation_type: &str) -> String {
    format!(
        r#"
            interface Repository<T> {{
              function save(self: Self, input: T, nested: Array<T?>?) -> T
            }}
            type Handler implements Repository<payments.User> {{}}
            impl Handler {{
              function save(self: Handler, input: {implementation_type}, nested: Array<{implementation_type}?>?) -> {implementation_type} {{
                return input
              }}
            }}
        "#
    )
}

#[test]
fn exact_interface_query_preserves_local_contract_and_nested_substitution() {
    let dependencies = contract_dependencies_with(&[("payments", "example.payments")]);
    let model = build_interface_model(&repository_source("payments.User"), &dependencies)
        .expect("exact interface conformance builds");
    let facts = model.interface_signatures();
    let method_key = SourceInterfaceMethodKey {
        interface: crate::SourceSymbolKey::new("api", "Repository"),
        method: "save".to_string(),
    };
    let raw = facts.requirement(&method_key).expect("exact requirement");
    assert!(matches!(
        &raw.return_type,
        PackageTypeRef::Local {
            local_type: TypeRefIr::TypeParam { name }
        } if name == "T"
    ));

    let conformance_key = SourceInterfaceConformanceKey {
        receiver: crate::SourceSymbolKey::new("api", "Handler"),
        interface: crate::SourceSymbolKey::new("api", "Repository"),
    };
    let conformance = facts
        .conformance(&conformance_key)
        .expect("validated conformance");
    let expected_id = contract_type_id("example.payments", "1.0.0", "User").unwrap();
    assert!(matches!(
        conformance.canonical_substitutions.get("T"),
        Some(PackageTypeRef::Contract { contract_type_id }) if contract_type_id == &expected_id
    ));
    assert!(matches!(
        conformance.canonical_substitutions.get("Self"),
        Some(PackageTypeRef::Local { .. })
    ));
    let method = facts
        .validated_method(&conformance_key, "save")
        .expect("validated method query");
    assert!(matches!(
        &method.exact_requirement.return_type,
        PackageTypeRef::Contract { contract_type_id } if contract_type_id == &expected_id
    ));
    assert!(matches!(
        &method.exact_requirement.parameters[2].ty,
        PackageTypeRef::Nullable { inner }
            if matches!(inner.as_ref(), PackageTypeRef::Container { name, arguments }
                if name == "Array"
                && matches!(arguments.as_slice(), [PackageTypeRef::Nullable { inner }]
                    if matches!(inner.as_ref(), PackageTypeRef::Contract { contract_type_id }
                        if contract_type_id == &expected_id)))
    ));
    assert!(matches!(method.receiver_type, PackageTypeRef::Local { .. }));
}

#[test]
fn different_aliases_for_the_same_contract_identity_conform_exactly() {
    let dependencies = contract_dependencies_with(&[
        ("payments", "example.payments"),
        ("billing", "example.payments"),
    ]);
    let model = build_interface_model(&repository_source("billing.User"), &dependencies)
        .expect("alias spelling must not participate in exact conformance");
    let key = SourceInterfaceConformanceKey {
        receiver: crate::SourceSymbolKey::new("api", "Handler"),
        interface: crate::SourceSymbolKey::new("api", "Repository"),
    };
    let method = model
        .interface_signatures()
        .validated_method(&key, "save")
        .expect("validated exact method");
    assert!(matches!(
        &method.exact_requirement.return_type,
        PackageTypeRef::Contract { .. }
    ));
    assert!(!matches!(
        &method.exact_requirement.return_type,
        PackageTypeRef::Local {
            local_type: TypeRefIr::ServiceSymbol { .. }
        }
    ));
}

#[test]
fn concrete_local_interface_types_remain_exact_local_facts() {
    let model = build_interface_model(
        r#"
            type Payload { value: string }
            interface LocalApi { function map(input: Payload) -> Payload }
            type Handler implements LocalApi {}
            impl Handler {
              function map(input: Payload) -> Payload { return input }
            }
        "#,
        &SourceDependencyAnalysisInput::default(),
    )
    .expect("local exact interface conformance builds");
    let method_key = SourceInterfaceMethodKey {
        interface: crate::SourceSymbolKey::new("api", "LocalApi"),
        method: "map".to_string(),
    };
    let requirement = model
        .interface_signatures()
        .requirement(&method_key)
        .expect("local exact requirement");
    assert!(matches!(
        &requirement.parameters[0].ty,
        PackageTypeRef::Local {
            local_type: TypeRefIr::LocalType { .. }
        }
    ));
    assert!(matches!(
        &requirement.return_type,
        PackageTypeRef::Local {
            local_type: TypeRefIr::LocalType { .. }
        }
    ));
}

#[test]
fn different_contract_identity_cannot_conform_through_alias_shaped_symbols() {
    let dependencies = contract_dependencies_with(&[
        ("payments", "example.payments"),
        ("accounts", "example.accounts"),
    ]);
    let error = build_interface_model(&repository_source("accounts.User"), &dependencies)
        .expect_err("different ContractTypeId must fail exact conformance")
        .to_string();
    assert!(
        error.contains("exact signature does not match interface"),
        "unexpected error: {error}"
    );
}

#[test]
fn interface_conformance_fails_closed_for_missing_method_and_receiver_mismatch() {
    let dependencies = contract_dependencies_with(&[("payments", "example.payments")]);
    let missing = build_interface_model(
        r#"
            interface Repository { function save(input: payments.User) -> payments.User }
            type Handler implements Repository {}
        "#,
        &dependencies,
    )
    .expect_err("missing method must fail")
    .to_string();
    assert!(missing.contains("method save is missing"));

    let receiver = build_interface_model(
        r#"
            interface Repository { function save(self: Self, input: payments.User) -> payments.User }
            type Handler implements Repository {}
            impl Handler {
              function save(self: string, input: payments.User) -> payments.User { return input }
            }
        "#,
        &dependencies,
    )
    .expect_err("wrong explicit receiver cannot satisfy interface requirement")
    .to_string();
    assert!(
        receiver.contains("receiver does not match its declared receiver type"),
        "unexpected error: {receiver}"
    );
}

#[test]
fn unrelated_contract_symbols_do_not_change_validated_interface_query() {
    let source = repository_source("billing.User");
    let base_dependencies = contract_dependencies_with(&[
        ("payments", "example.payments"),
        ("billing", "example.payments"),
    ]);
    let extra_dependencies = contract_dependencies_with(&[
        ("payments", "example.payments"),
        ("billing", "example.payments"),
        ("unused", "example.unused"),
    ]);
    let base = build_interface_model(&source, &base_dependencies).expect("base exact facts build");
    let extra = build_interface_model(&source, &extra_dependencies)
        .expect("unrelated dependency does not affect exact facts");
    assert_eq!(base.interface_signatures(), extra.interface_signatures());
}

const PACKAGE_INTERFACE_ID: &str = "example.reader";

fn package_interface_dependency() -> (PackageSourceModel, Vec<FileIrUnit>) {
    let package_api = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
        "Reader", "api", "Reader",
    )]);
    let package_model = build_model_with_publication_api(
        r#"
            interface Reader<T> {
              function read(self: Self, fallback: T) -> T
            }
        "#,
        &SourceDependencyAnalysisInput::default(),
        &BTreeMap::new(),
        &[],
        &package_api,
    )
    .expect("package interface source model builds");
    let mut unit = FileIrUnit::empty("api", "reader-package");
    unit.declarations.interfaces.insert(
        "Reader".to_string(),
        InterfaceDeclIr {
            name: "Reader".to_string(),
            type_params: vec!["T".to_string()],
            operations: vec![InterfaceOperationIr {
                name: "read".to_string(),
                type_params: Vec::new(),
                params: vec![
                    FunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: TypeRefIr::native("Self"),
                    },
                    FunctionTypeParamIr {
                        name: "fallback".to_string(),
                        ty: TypeRefIr::TypeParam {
                            name: "T".to_string(),
                        },
                    },
                ],
                return_type: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: None,
            }],
            source_span: None,
        },
    );
    (package_model, vec![unit])
}

fn build_model_with_package_interface(
    source: &str,
    package_model: &PackageSourceModel,
    package_units: &[FileIrUnit],
) -> Result<PackageSourceModel, SourceCompileError> {
    let package_facts = vec![SourceCompilePackageFacts::new(
        PACKAGE_INTERFACE_ID,
        "1.0.0",
        Vec::new(),
        package_model,
        package_units,
    )];
    let mut dependency = PackageDependency::id(PACKAGE_INTERFACE_ID);
    dependency.alias = Some("pkg".to_string());
    build_model_with_publication_api_and_package_facts(
        source,
        &SourceDependencyAnalysisInput::default(),
        &BTreeMap::from([("pkg".to_string(), vec![String::new()])]),
        &[dependency],
        Some(&package_facts),
        &PublicationApiSpec::empty(),
    )
}

#[test]
fn package_interface_conformance_stays_owned_by_canonical_package_facts() {
    let (package_model, package_units) = package_interface_dependency();
    let model = build_model_with_package_interface(
        r#"
            type Host implements pkg.Reader<string> { value: string }
            impl Host {
              function read(fallback: string) -> string { return fallback }
            }
            function make_box(host: Host) -> void {
              let reader = host as pkg.Reader<string>
            }
        "#,
        &package_model,
        &package_units,
    )
    .expect("validated package interface remains outside source exact conformance ownership");

    assert!(model.interface_signatures().conformances().next().is_none());

    let context = TypeResolutionContext::source("api");
    let actual = model
        .type_resolution()
        .resolve_type_text("Host", &context)
        .expect("local receiver resolves");
    let expected = model
        .type_resolution()
        .resolve_type_text("pkg.Reader<string>", &context)
        .expect("package interface resolves");
    let conformance = model
        .type_resolution()
        .local_any_interface_conformance_for_boxing(&actual, &expected, &context)
        .expect("package conformance query succeeds")
        .expect("canonical package facts validate conformance");
    let TypeRefIr::PackageSymbol { symbol } =
        serde_json::from_str::<TypeRefIr>(&conformance.interface.interface_abi_id)
            .expect("package interface identity decodes")
    else {
        panic!("package interface identity must remain a PackageSymbol");
    };
    assert_eq!(symbol.symbol_path, "Reader");
    assert!(matches!(
        symbol.package,
        skiff_artifact_model::PackageRefIr::PackageId { package_id }
            if package_id == PACKAGE_INTERFACE_ID
    ));
    assert_eq!(
        conformance.interface.canonical_type_args,
        vec![TypeRefIr::native("string")]
    );
    assert_eq!(conformance.slots.len(), 1);
    assert_eq!(
        conformance.slots[0].params[1].ty,
        TypeRefIr::native("string")
    );
    assert_eq!(
        conformance.slots[0].return_type,
        TypeRefIr::native("string")
    );
}

#[test]
fn package_interface_entries_still_fail_closed_before_source_owner_handoff() {
    let (package_model, package_units) = package_interface_dependency();
    let mismatch = build_model_with_package_interface(
        r#"
            type Host implements pkg.Reader<string> { value: string }
            impl Host {
              function read(fallback: number) -> string { return "bad" }
            }
            function make_box(host: Host) -> void {
              let reader = host as pkg.Reader<string>
            }
        "#,
        &package_model,
        &package_units,
    )
    .expect_err("package interface signature mismatch must fail")
    .to_string();
    assert!(
        mismatch.contains("does not explicitly implement interface"),
        "unexpected mismatch error: {mismatch}"
    );

    let unknown = build_model_with_package_interface(
        "type Host implements pkg.Missing { value: string }",
        &package_model,
        &package_units,
    )
    .expect_err("unknown package interface must fail")
    .to_string();
    assert!(
        unknown.contains("implements entry `pkg.Missing` is not an interface"),
        "unexpected unknown interface error: {unknown}"
    );
}
