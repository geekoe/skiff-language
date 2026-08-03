use std::path::{Path, PathBuf};

use compiler_input_model::{
    PackageCompilePolicy, PackageDependency, PublicationApiEntry,
    PublicationApiPublicInstanceEntry, PublicationApiSpec,
};
use skiff_artifact_identity::package_schema_type_id;
use skiff_artifact_model::{
    ContractLiteral, ContractTypeRef, FileIrUnit, FunctionTypeParamIr, InterfaceDeclIr,
    InterfaceOperationIr, PackageTypeRef, TypeRefIr,
};
use skiff_compiler_input::CompilerPlatformSources;

use crate::{
    build_package_from_parsed_sources_with_dependency_analysis,
    contract_dependency_test_fixture::{
        resolved_contract_fixture, resolved_nullable_field_contract_fixture,
    },
    parsed_sources::parse_publication_sources,
    source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageSourceModel, SourceCompileError,
    SourceCompilePackageFacts,
};

use super::*;

mod executable_signatures;
mod interface_signatures;

fn fixture_type_id(
    service_id: &str,
    stable_key: &str,
) -> skiff_artifact_model::PackageSchemaTypeId {
    let descriptor = skiff_artifact_model::PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: skiff_artifact_model::ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("value".to_string(), ContractTypeRef::builtin("string"))]),
        },
    };
    package_schema_type_id(&format!("{service_id}.package"), stable_key, &descriptor).unwrap()
}

#[test]
fn exported_signature_preserves_contract_nominal_nested_types_and_local_domain() {
    let user_id = fixture_type_id("example.payments", "User");
    let dependency_analysis = contract_dependencies();
    let model = build_model(
        r#"
                type User { value: string }
                alias RemoteUser = payments.User

                function submit(
                    local: User,
                    remote: payments.User,
                    aliased: RemoteUser,
                    nested: Array<payments.User?>?
                ) -> payments.User {
                    return remote
                }

                function privateSink(input: payments.User) -> void {}

                function suspendedHelper(input: payments.User) -> void {
                    dispatch privateSink(input)
                }
            "#,
        &dependency_analysis,
        &BTreeMap::new(),
    )
    .expect("contract-aware source signature builds");

    let signatures = model.callable_signatures();
    assert_eq!(signatures.iter().count(), 1);
    let signature = signatures.signature("submit").expect("submit signature");
    assert!(matches!(
        &signature.parameters[0].ty,
        PackageTypeRef::Local {
            local_type: TypeRefIr::LocalType { .. }
        }
    ));
    assert!(matches!(
        &signature.parameters[1].ty,
        PackageTypeRef::PackageSchema { package_schema_type_id, .. } if package_schema_type_id == &user_id
    ));
    assert!(matches!(
        &signature.parameters[2].ty,
        PackageTypeRef::PackageSchema { package_schema_type_id, .. } if package_schema_type_id == &user_id
    ));
    assert!(matches!(
            &signature.parameters[3].ty,
            PackageTypeRef::Nullable { inner }
            if matches!(inner.as_ref(), PackageTypeRef::Container { name, arguments }
                if name == "Array"
                && matches!(arguments.as_slice(), [PackageTypeRef::Nullable { inner }]
                    if matches!(inner.as_ref(), PackageTypeRef::PackageSchema { package_schema_type_id, .. }
                        if package_schema_type_id == &user_id)))
    ));
    assert!(matches!(
        &signature.return_type,
        PackageTypeRef::PackageSchema { package_schema_type_id, .. } if package_schema_type_id == &user_id
    ));
    assert!(!signature.may_suspend);

    let executable_signatures = model.executable_signatures();
    assert_eq!(executable_signatures.iter().count(), 3);
    let private_sink = executable_signatures
        .signature(&crate::SourceSymbolKey::new("api", "privateSink"))
        .expect("private helper receives an exact signature fact");
    assert!(matches!(
        private_sink.receiver,
        SourceExecutableReceiver::None
    ));
    assert!(matches!(
        &private_sink.parameters[0].ty,
        PackageTypeRef::PackageSchema { package_schema_type_id, .. } if package_schema_type_id == &user_id
    ));
    let suspended = executable_signatures
        .signature(&crate::SourceSymbolKey::new("api", "suspendedHelper"))
        .expect("private suspending helper receives an exact signature fact");
    assert!(suspended.may_suspend);
}

#[test]
fn service_api_nominal_record_uses_ordinary_constructor_and_field_paths() {
    let dependency_analysis = contract_dependencies();
    let model = build_model(
        r#"
            function submit(input: payments.User) -> payments.User {
                let copied = payments.User { value: input.value }
                payments/submit(copied)
                return copied
            }
        "#,
        &dependency_analysis,
        &BTreeMap::new(),
    )
    .expect("service API records construct and expose fields through the shared type model");

    let signature = model
        .callable_signatures()
        .signature("submit")
        .expect("public signature");
    let expected = fixture_type_id("example.payments", "User");
    assert!(matches!(
        &signature.return_type,
        PackageTypeRef::PackageSchema { package_schema_type_id, .. } if package_schema_type_id == &expected
    ));
    assert!(matches!(
        model
            .resolved_call_targets()
            .iter()
            .map(|(_, target)| target)
            .find(|target| matches!(target, crate::ResolvedCallTarget::ContractOperation { .. })),
        Some(crate::ResolvedCallTarget::ContractOperation { .. })
    ));
}

#[test]
fn service_api_nominal_record_rejects_unknown_constructor_and_read_fields() {
    let dependency_analysis = contract_dependencies();
    for source in [
        r#"
            function submit(input: payments.User) -> payments.User {
                return payments.User { missing: input.value }
            }
        "#,
        r#"
            function submit(input: payments.User) -> string {
                return input.missing
            }
        "#,
    ] {
        let error = build_model(source, &dependency_analysis, &BTreeMap::new())
            .expect_err("unknown service API fields fail closed")
            .to_string();
        assert!(
            error.contains("missing") || error.contains("field"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn stable_nullable_field_paths_narrow_local_and_imported_nominal_records() {
    build_model(
        r#"
            type ToolError { code: string }
            type ToolResult { error: ToolError? }

            function consume(error: ToolError) -> string { return error.code }

            function submit(result: ToolResult) -> string {
                if result.error != null {
                    return consume(result.error)
                }
                return ""
            }
        "#,
        &SourceDependencyAnalysisInput::default(),
        &BTreeMap::new(),
    )
    .expect("a stable local nominal field must carry its non-null refinement to the read");

    let dependency_analysis = nullable_field_contract_dependencies();
    build_model(
        r#"
            function consume(error: agent.ToolError) -> string { return error.code }

            function submit(result: agent.ToolResult) -> string {
                if result.error != null {
                    return consume(result.error)
                }
                return ""
            }
        "#,
        &dependency_analysis,
        &BTreeMap::new(),
    )
    .expect("an imported PackageSchema field must carry its exact non-null projection to the read");
}

#[test]
fn stable_nullable_field_path_narrowing_fails_closed_after_invalidation_or_for_other_paths() {
    let cases = [
        (
            r#"
                type ToolError { code: string }
                type ToolResult { error: ToolError? }
                type State { current: ToolResult }

                function consume(error: ToolError) -> string { return error.code }

                function submit(state: State) -> string {
                    if state.current.error != null {
                        state.current = ToolResult { error: null }
                        return consume(state.current.error)
                    }
                    return ""
                }
            "#,
            "a write to the stable path prefix must invalidate its child refinement",
        ),
        (
            r#"
                type ToolError { code: string }
                type ToolResult { error: ToolError? }

                function maybe() -> ToolResult {
                    return ToolResult { error: null }
                }
                function consume(error: ToolError) -> string { return error.code }

                function submit() -> string {
                    if maybe().error != null {
                        return consume(maybe().error)
                    }
                    return ""
                }
            "#,
            "a call result is not a stable path",
        ),
        (
            r#"
                type ToolError { code: string }
                type ToolResult { error: ToolError? }

                function consume(error: ToolError) -> string { return error.code }

                function submit(left: ToolResult, right: ToolResult) -> string {
                    if left.error != null {
                        return consume(right.error)
                    }
                    return ""
                }
            "#,
            "a refinement must not leak to an unrelated path",
        ),
    ];

    for (source, label) in cases {
        let error = build_model(
            source,
            &SourceDependencyAnalysisInput::default(),
            &BTreeMap::new(),
        )
        .expect_err(label)
        .to_string();
        assert!(
            error.contains("type mismatch") || error.contains("nullable"),
            "{label}: unexpected diagnostic:\n{error}"
        );
    }

    let dependency_analysis = nullable_field_contract_dependencies();
    let error = build_model(
        r#"
            function consume(error: agent.ToolError) -> string { return error.code }

            function submit(result: agent.ToolResult) -> string {
                if result.error != null {
                    result.error = null
                    return consume(result.error)
                }
                return ""
            }
        "#,
        &dependency_analysis,
        &BTreeMap::new(),
    )
    .expect_err("a write must invalidate an imported PackageSchema field refinement")
    .to_string();
    assert!(
        error.contains("canonical type identity mismatch"),
        "unexpected imported invalidation diagnostic:\n{error}"
    );
}

#[test]
fn service_alias_selects_validated_package_schema_record() {
    let dependencies = contract_dependencies();
    let record = dependencies
        .public_package_type_by_stable_key("payments", "User")
        .unwrap();
    assert_eq!(record.package_id, "example.payments.package");
    assert_eq!(record.stable_schema_key, "User");
}

#[test]
fn public_instance_operations_receive_exact_source_owned_signatures() {
    let user_id = fixture_type_id("example.payments", "User");
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
              function submit(
                input: payments.User,
                nested: Array<payments.User?>?
              ) -> payments.User
            }
            type Handler implements PublicApi {}
            impl Handler {
              function submit(
                input: payments.User,
                nested: Array<payments.User?>?
              ) -> payments.User {
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
    .expect("public instance signatures build from source facts");

    let signatures = model.callable_signatures();
    assert_eq!(signatures.iter().count(), 1);
    let signature = signatures
        .signature("handler.submit")
        .expect("derived public instance operation signature");
    assert!(matches!(
        &signature.parameters[0].ty,
        PackageTypeRef::PackageSchema { package_schema_type_id, .. } if package_schema_type_id == &user_id
    ));
    assert!(matches!(
        &signature.parameters[1].ty,
        PackageTypeRef::Nullable { inner }
            if matches!(inner.as_ref(), PackageTypeRef::Container { name, arguments }
                if name == "Array"
                && matches!(arguments.as_slice(), [PackageTypeRef::Nullable { inner }]
                    if matches!(inner.as_ref(), PackageTypeRef::PackageSchema { package_schema_type_id, .. }
                        if package_schema_type_id == &user_id)))
    ));
    assert!(matches!(
        &signature.return_type,
        PackageTypeRef::PackageSchema { package_schema_type_id, .. } if package_schema_type_id == &user_id
    ));

    let executable = model
        .executable_signatures()
        .signature(&crate::SourceSymbolKey::new("api", "Handler.submit"))
        .expect("public instance implementation has one exact executable fact");
    assert!(matches!(
        &executable.receiver,
        SourceExecutableReceiver::Implicit {
            ty: PackageTypeRef::Local { .. }
        }
    ));
    assert_eq!(executable.parameters.len(), 2);
}

#[test]
fn public_type_and_public_instance_publish_impl_method_only_through_instance() {
    let dependency_analysis = contract_dependencies();
    let publication_api = PublicationApiSpec::new(
        vec![
            PublicationApiEntry::for_source("Handler", "api", "Handler"),
            PublicationApiEntry::for_source("submit", "api", "submit"),
        ],
        vec![PublicationApiPublicInstanceEntry::for_source(
            "handler",
            "root.api.handler",
            ["root.api.PublicApi"],
        )
        .unwrap()],
        None,
    );
    let model = build_model_with_publication_api(
        r#"
            interface PublicApi {
              function submit(input: string) -> string
            }
            type Handler implements PublicApi {}
            impl Handler {
              function submit(self: Handler, input: string) -> string {
                return input
              }
            }
            const handler: Handler = Handler {}
            function submit(input: string) -> string { return input }
        "#,
        &dependency_analysis,
        &BTreeMap::new(),
        &[],
        &publication_api,
    )
    .expect("publication ownership must select exact callable paths");

    let signatures = model.callable_signatures();
    assert_eq!(signatures.iter().count(), 2);
    assert!(signatures.signature("handler.submit").is_some());
    assert!(signatures.signature("submit").is_some());
    assert!(signatures.signature("Handler.submit").is_none());
}

#[test]
fn unknown_service_package_type_fails_closed() {
    let dependency_analysis = contract_dependencies();
    let error = build_model(
        "function submit(input: payments.Missing) -> void {}",
        &dependency_analysis,
        &BTreeMap::new(),
    )
    .expect_err("unknown public package type must fail source compilation")
    .to_string();
    assert!(error.contains("Missing"), "unexpected error: {error}");
}

#[test]
fn package_and_contract_alias_conflict_fails_before_type_context_can_choose() {
    let dependency_analysis = contract_dependencies();
    let package_aliases = BTreeMap::from([("payments".to_string(), Vec::new())]);
    let error = build_model(
        "function submit(input: payments.User) -> void {}",
        &dependency_analysis,
        &package_aliases,
    )
    .expect_err("cross-kind alias conflict must fail")
    .to_string();
    assert!(
        error.contains("dependency alias `payments` is declared by both a package and a contract"),
        "unexpected error: {error}"
    );
}

#[test]
fn package_dependency_qualified_type_keeps_existing_package_symbol_resolution() {
    let dependency_analysis = SourceDependencyAnalysisInput::default();
    let package_aliases = BTreeMap::from([("tools".to_string(), Vec::new())]);
    let mut dependency = PackageDependency::id("example.tools");
    dependency.alias = Some("tools".to_string());
    let model = build_model_with_package_dependencies(
        "function submit(input: tools.User) -> void {}",
        &dependency_analysis,
        &package_aliases,
        &[dependency],
    )
    .expect("package qualified type keeps existing resolution");
    let signature = model
        .callable_signatures()
        .signature("submit")
        .expect("submit signature");
    assert!(matches!(
        &signature.parameters[0].ty,
        PackageTypeRef::Local {
            local_type: TypeRefIr::PackageSymbol { symbol }
        } if matches!(
            &symbol.package,
            skiff_artifact_model::PackageRefIr::Dependency { dependency_ref }
                if dependency_ref == "tools"
        ) && symbol.symbol_path == "User"
    ));
}

#[test]
fn private_unknown_contract_type_is_also_rejected() {
    let dependency_analysis = contract_dependencies();
    let source = CompilerSourceFile::parse(
        PathBuf::from("api.skiff"),
        "api".to_string(),
        true,
        false,
        "function private(input: payments.Missing) -> void {}".to_string(),
        "api.skiff",
    )
    .expect("fixture parses");
    let parsed_sources =
        parse_publication_sources(Path::new("/tmp/contract-type-resolution"), &[source])
            .expect("fixture source facts build");
    let error = validate_contract_type_uses(&parsed_sources, &dependency_analysis)
        .expect_err("private unknown contract type must fail closed");
    assert!(error.contains("no contract type stable key `Missing`"));
}

#[test]
fn inline_contract_shapes_have_no_lossy_package_type_fallback() {
    let inline = [
        ContractTypeRef::Record {
            fields: BTreeMap::new(),
        },
        ContractTypeRef::StructuralUnion {
            variants: vec![ContractTypeRef::builtin("string")],
        },
        ContractTypeRef::Literal {
            value: ContractLiteral::String {
                value: "ok".to_string(),
            },
        },
    ];
    for ty in inline {
        let error = package_type_ref_from_validated_contract_ref(&ty)
            .expect_err("inline contract shape must fail closed");
        assert!(error.contains("no exact PackageTypeRef representation"));
    }
}

fn build_model(
    source: &str,
    dependency_analysis: &SourceDependencyAnalysisInput,
    package_aliases: &BTreeMap<String, Vec<String>>,
) -> Result<PackageSourceModel, SourceCompileError> {
    build_model_with_package_dependencies(source, dependency_analysis, package_aliases, &[])
}

fn build_model_with_package_dependencies(
    source: &str,
    dependency_analysis: &SourceDependencyAnalysisInput,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_dependencies: &[PackageDependency],
) -> Result<PackageSourceModel, SourceCompileError> {
    let publication_api = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
        "submit", "api", "submit",
    )]);
    build_model_with_publication_api(
        source,
        dependency_analysis,
        package_aliases,
        package_dependencies,
        &publication_api,
    )
}

fn build_model_with_publication_api(
    source: &str,
    dependency_analysis: &SourceDependencyAnalysisInput,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_dependencies: &[PackageDependency],
    publication_api: &PublicationApiSpec,
) -> Result<PackageSourceModel, SourceCompileError> {
    build_model_with_publication_api_and_package_facts(
        source,
        dependency_analysis,
        package_aliases,
        package_dependencies,
        None,
        publication_api,
    )
}

fn build_model_with_publication_api_and_package_facts(
    source: &str,
    dependency_analysis: &SourceDependencyAnalysisInput,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_dependencies: &[PackageDependency],
    package_facts: Option<&[SourceCompilePackageFacts<'_>]>,
    publication_api: &PublicationApiSpec,
) -> Result<PackageSourceModel, SourceCompileError> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    crate::prelude_registry::initialize_prelude_registry(
        &CompilerPlatformSources::new(&platform_root).expect("platform sources load"),
    )
    .expect("prelude registry initializes");
    let source = CompilerSourceFile::parse(
        PathBuf::from("api.skiff"),
        "api".to_string(),
        true,
        false,
        source.to_string(),
        "api.skiff",
    )
    .expect("fixture parses");
    let production_sources = vec![source];
    let parsed_sources = parse_publication_sources(
        Path::new("/tmp/contract-type-resolution"),
        &production_sources,
    )
    .expect("fixture source facts build");
    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: Path::new("/tmp/contract-type-resolution"),
            publication_api: Some(publication_api),
            package_aliases,
            package_dependencies,
            package_facts,
            package_artifacts: None,
            policy: PackageCompilePolicy::new("example.com/contract-type-resolution"),
        },
        dependency_analysis,
    )
}

fn contract_dependencies() -> SourceDependencyAnalysisInput {
    SourceDependencyAnalysisInput::new(
        Vec::new(),
        [resolved_contract_fixture(
            "payments",
            "example.payments",
            "submit",
            "User",
            "Secret",
        )],
    )
    .unwrap()
}

fn nullable_field_contract_dependencies() -> SourceDependencyAnalysisInput {
    SourceDependencyAnalysisInput::new(
        Vec::new(),
        [resolved_nullable_field_contract_fixture(
            "agent",
            "example.agent",
            "run",
            "ToolResult",
            "error",
            "ToolError",
        )],
    )
    .unwrap()
}
