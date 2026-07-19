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
