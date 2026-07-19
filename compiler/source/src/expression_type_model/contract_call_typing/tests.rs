use std::{collections::BTreeMap, path::Path};

use compiler_input_model::{PackageCompilePolicy, PublicationApiSpec};
use skiff_artifact_identity::{
    assign_service_contract_identities, contract_operation_id, contract_type_id,
};
use skiff_artifact_model::{BoundaryCancellationContract, BoundaryStreamContract, ContractTypeRef};
use skiff_compiler_input::ResolvedContractDependency;

use crate::{
    build_package_from_parsed_sources_with_dependency_analysis,
    contract_dependency_test_fixture::{contract_fixture, requirement},
    parsed_sources::parse_publication_sources,
    source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageSourceModel, ResolvedCallTarget,
    SourceDependencyAnalysisInput,
};

#[test]
fn valid_contract_call_is_typed_before_target_facts_are_published() {
    let dependencies = dependencies(&[("payments", "example.payments")]);
    let model = build_model(
        r#"
            function entry(input: payments.User) -> payments.User {
                return payments/submit(input)
            }
        "#,
        &dependencies,
    )
    .expect("valid descriptor argument and return should type-check");

    let targets = model
        .resolved_call_targets()
        .iter()
        .filter(|(_, target)| matches!(target, ResolvedCallTarget::ContractOperation { .. }))
        .count();
    assert_eq!(
        targets, 1,
        "only a typed-successful call publishes a target"
    );
}

#[test]
fn unknown_alias_and_operation_fail_source_compilation() {
    let dependencies = dependencies(&[("payments", "example.payments")]);
    for (call, expected) in [
        ("missing/submit(input)", "missing"),
        (
            "payments/missing(input)",
            "no operation stable key `missing`",
        ),
    ] {
        let error = build_model(
            &format!("function entry(input: payments.User) -> void {{ {call} }}"),
            &dependencies,
        )
        .expect_err("unknown contract target must fail before target facts")
        .to_string();
        assert!(error.contains(expected), "unexpected error: {error}");
    }
}

#[test]
fn dotted_contract_call_is_not_a_dependency_call_compatibility_spelling() {
    let dependencies = dependencies(&[("payments", "example.payments")]);
    let error = build_model(
        r#"
            function entry(input: payments.User) -> payments.User {
                return payments.submit(input)
            }
        "#,
        &dependencies,
    )
    .expect_err("dot-call spelling must not resolve through a contract alias")
    .to_string();
    assert!(
        error.contains("payments") && error.contains("submit"),
        "unexpected error: {error}"
    );
}

#[test]
fn wrong_arity_and_argument_type_fail_descriptor_checking() {
    let dependencies = dependencies(&[("payments", "example.payments")]);
    for (body, expected) in [
        ("payments/submit()", "arity mismatch"),
        (
            "payments/submit(\"not a contract user\")",
            "argument 1 type mismatch",
        ),
    ] {
        let error = build_model(
            &format!("function entry() -> void {{ {body} }}"),
            &dependencies,
        )
        .expect_err("invalid contract argument must fail source typing")
        .to_string();
        assert!(error.contains(expected), "unexpected error: {error}");
    }
}

#[test]
fn contract_call_return_use_is_checked_in_source() {
    let dependencies = dependencies(&[("payments", "example.payments")]);
    let error = build_model(
        r#"
            function entry(input: payments.User) -> string {
                return payments/submit(input)
            }
        "#,
        &dependencies,
    )
    .expect_err("contract return must not flow into an incompatible source return")
    .to_string();
    assert!(
        error.contains("return type mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn contract_nominals_compare_by_contract_type_id() {
    let dependencies = dependencies(&[
        ("payments", "example.payments"),
        ("accounts", "example.accounts"),
    ]);
    let error = build_model(
        r#"
            function entry(input: accounts.User) -> void {
                payments/submit(input)
            }
        "#,
        &dependencies,
    )
    .expect_err("structurally equal nominals from different contracts must not match")
    .to_string();
    assert!(
        error.contains("argument 1 type mismatch")
            && error.contains("payments.User")
            && error.contains("accounts.User"),
        "unexpected error: {error}"
    );
}

#[test]
fn builtin_container_and_nullable_contract_types_compare_recursively() {
    let mut payments = callable_contract("example.payments");
    let payments_user = contract_type_id("example.payments", "1.0.0", "User").unwrap();
    let operation = payments.operations.values_mut().next().unwrap();
    operation.contract.parameters[0].ty = ContractTypeRef::Builtin {
        name: "Array".to_string(),
        arguments: vec![ContractTypeRef::Nullable {
            inner: Box::new(ContractTypeRef::contract(payments_user)),
        }],
    };
    operation.contract.return_value.ty = ContractTypeRef::builtin("bool");
    assign_service_contract_identities(&mut payments).unwrap();
    let accounts = callable_contract("example.accounts");
    let dependencies = SourceDependencyAnalysisInput::new(
        Vec::new(),
        [
            ResolvedContractDependency::validated(requirement("payments", &payments), payments)
                .unwrap(),
            ResolvedContractDependency::validated(requirement("accounts", &accounts), accounts)
                .unwrap(),
        ],
    )
    .unwrap();

    build_model(
        r#"
            function entry(input: Array<payments.User?>) -> bool {
                return payments/submit(input)
            }
        "#,
        &dependencies,
    )
    .expect("nested canonical contract type should match recursively");

    let error = build_model(
        r#"
            function entry(input: Array<accounts.User?>) -> void {
                payments/submit(input)
            }
        "#,
        &dependencies,
    )
    .expect_err("nested foreign contract nominal must not match")
    .to_string();
    assert!(
        error.contains("argument 1 type mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn may_suspend_unary_contract_call_uses_the_ordinary_source_call_form() {
    let mut contract = callable_contract("example.payments");
    let operation = contract.operations.values_mut().next().unwrap();
    operation.contract.may_suspend = true;
    operation.contract.cancellation = BoundaryCancellationContract::Cooperative;
    assign_service_contract_identities(&mut contract).unwrap();
    let dependencies = SourceDependencyAnalysisInput::new(
        Vec::new(),
        [
            ResolvedContractDependency::validated(requirement("payments", &contract), contract)
                .unwrap(),
        ],
    )
    .unwrap();

    build_model(
        r#"
            function entry(input: payments.User) -> payments.User {
                return payments/submit(input)
            }
        "#,
        &dependencies,
    )
    .expect("the language models suspension implicitly on ordinary calls");
}

#[test]
fn unsupported_generic_and_stream_call_forms_fail_source_typing() {
    let dependencies = dependencies(&[("payments", "example.payments")]);
    let generic_error = build_model(
        r#"
            function entry(input: payments.User) -> void {
                payments/submit<payments.User>(input)
            }
        "#,
        &dependencies,
    )
    .expect_err("contract operations do not expose source generics")
    .to_string();
    assert!(
        generic_error.contains("does not accept source type arguments"),
        "unexpected error: {generic_error}"
    );

    let mut streaming = callable_contract("example.streaming");
    let operation = streaming.operations.values_mut().next().unwrap();
    operation.contract.stream = BoundaryStreamContract::ServerStream {
        item_type: operation.contract.return_value.ty.clone(),
        item_value_plan: operation.contract.return_value.value_plan.clone(),
    };
    assign_service_contract_identities(&mut streaming).unwrap();
    let streaming_dependencies = SourceDependencyAnalysisInput::new(
        Vec::new(),
        [
            ResolvedContractDependency::validated(requirement("streaming", &streaming), streaming)
                .unwrap(),
        ],
    )
    .unwrap();
    let stream_error = build_model(
        r#"
            function entry(input: streaming.User) -> void {
                streaming/submit(input)
            }
        "#,
        &streaming_dependencies,
    )
    .expect_err("server stream descriptor cannot use the unary expression form")
    .to_string();
    assert!(
        stream_error.contains("stream contract unsupported by unary source calls"),
        "unexpected error: {stream_error}"
    );
}

#[test]
fn closure_only_contract_results_keep_nominal_identity_for_pass_through_calls() {
    let payments = closure_passthrough_contract("example.payments");
    let accounts = closure_passthrough_contract("example.accounts");
    let dependencies = SourceDependencyAnalysisInput::new(
        Vec::new(),
        [
            ResolvedContractDependency::validated(requirement("payments", &payments), payments)
                .unwrap(),
            ResolvedContractDependency::validated(requirement("accounts", &accounts), accounts)
                .unwrap(),
        ],
    )
    .unwrap();

    build_model(
        r#"
            function entry() -> bool {
                const secret = payments/fetch()
                return payments/consume(secret)
            }
        "#,
        &dependencies,
    )
    .expect("closure-only result should retain its typed contract identity internally");

    let error = build_model(
        r#"
            function entry() -> bool {
                const secret = accounts/fetch()
                return payments/consume(secret)
            }
        "#,
        &dependencies,
    )
    .expect_err("closure-only nominals from different contracts must not match")
    .to_string();
    assert!(
        error.contains("argument 1 type mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn inline_contract_operation_shapes_fail_closed() {
    let mut contract = callable_contract("example.payments");
    let operation = contract.operations.values_mut().next().unwrap();
    operation.contract.parameters[0].ty = ContractTypeRef::StructuralUnion {
        variants: vec![
            ContractTypeRef::builtin("number"),
            ContractTypeRef::builtin("string"),
        ],
    };
    assign_service_contract_identities(&mut contract).unwrap();
    let dependencies = SourceDependencyAnalysisInput::new(
        Vec::new(),
        [
            ResolvedContractDependency::validated(requirement("payments", &contract), contract)
                .unwrap(),
        ],
    )
    .unwrap();

    let error = build_model(
        r#"
            function entry(input: payments.User) -> void {
                payments/submit(input)
            }
        "#,
        &dependencies,
    )
    .expect_err("inline contract shape has no source typing fallback")
    .to_string();
    assert!(
        error.contains("unsupported inline contract shape"),
        "unexpected error: {error}"
    );
}

fn build_model(
    source: &str,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> Result<PackageSourceModel, crate::SourceCompileError> {
    let source = CompilerSourceFile::parse(
        "api.skiff".into(),
        "api".to_string(),
        true,
        false,
        source.to_string(),
        "api.skiff",
    )
    .expect("fixture parses");
    let production_sources = vec![source];
    let parsed_sources = parse_publication_sources(
        Path::new("/tmp/contract-call-type-checking"),
        &production_sources,
    )
    .expect("fixture source facts build");
    let publication_api = PublicationApiSpec::from_entries(Vec::new());
    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: Path::new("/tmp/contract-call-type-checking"),
            publication_api: Some(&publication_api),
            package_aliases: &BTreeMap::new(),
            package_dependencies: &[],
            package_facts: None,
            policy: PackageCompilePolicy::new("example.com/contract-call-type-checking"),
        },
        dependency_analysis,
    )
}

fn dependencies(aliases: &[(&str, &str)]) -> SourceDependencyAnalysisInput {
    SourceDependencyAnalysisInput::new(
        Vec::new(),
        aliases.iter().map(|(alias, service_id)| {
            let contract = callable_contract(service_id);
            ResolvedContractDependency::validated(requirement(alias, &contract), contract).unwrap()
        }),
    )
    .unwrap()
}

fn callable_contract(service_id: &str) -> skiff_artifact_model::ServiceContract {
    let mut contract = contract_fixture(service_id, "1.0.0", "submit", "User", "InternalResult");
    let user_id = contract_type_id(service_id, "1.0.0", "User").unwrap();
    let operation = contract.operations.values_mut().next().unwrap();
    operation.contract.parameters[0].ty = ContractTypeRef::contract(user_id.clone());
    operation.contract.return_value.ty = ContractTypeRef::contract(user_id);
    assign_service_contract_identities(&mut contract).unwrap();
    contract
}

fn closure_passthrough_contract(service_id: &str) -> skiff_artifact_model::ServiceContract {
    let mut contract = contract_fixture(service_id, "1.0.0", "fetch", "User", "InternalResult");
    let internal_id = contract_type_id(service_id, "1.0.0", "InternalResult").unwrap();
    let fetch = contract.operations.values_mut().next().unwrap();
    let parameter_plan = fetch.contract.parameters[0].value_plan.clone();
    fetch.contract.parameters.clear();
    fetch.contract.return_value.ty = ContractTypeRef::contract(internal_id.clone());

    let consume_id = contract_operation_id(service_id, "1.0.0", "consume").unwrap();
    let mut consume = fetch.clone();
    consume.operation_id = consume_id.clone();
    consume.stable_key = "consume".to_string();
    consume
        .contract
        .parameters
        .push(skiff_artifact_model::BoundaryParameter {
            name: "secret".to_string(),
            ty: ContractTypeRef::contract(internal_id),
            value_plan: parameter_plan,
        });
    consume.contract.return_value.ty = ContractTypeRef::builtin("bool");
    contract.operations.insert(consume_id, consume);
    assign_service_contract_identities(&mut contract).unwrap();
    contract
}
