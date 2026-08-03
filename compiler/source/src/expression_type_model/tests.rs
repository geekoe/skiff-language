use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    build_package_from_parsed_sources_with_dependency_analysis,
    contract_dependency_test_fixture::resolved_contract_fixture,
    parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
    publication_db_metadata_index, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageCompilePolicy, PackageDependencyAnalysisFacts,
    PackageDependencyCallableAnalysis, PublicationDbMetadataIndex, PublicationTypeSymbolIndex,
    SourceDependencyAnalysisInput,
};
use skiff_artifact_model::{
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary,
    CallableProvenanceUnknownReason, CallableSemanticFacts, PackageBuildId, PackageCallableId,
    PackageCallableParameter, PackageCallableSignature, PackageLocalAbiIdentity, PackageTypeRef,
};
use skiff_compiler_input::CompilerPlatformSources;

use super::*;

const ANY_INTERFACE_MODULE: &str = "internal.any_interface";

fn expression_type_result(
    source_text: &str,
) -> Result<ExpressionTypeModel, ExpressionTypeModelBuildError> {
    expression_type_result_with_source_role(source_text, false)
}

fn test_expression_type_result(
    source_text: &str,
) -> Result<ExpressionTypeModel, ExpressionTypeModelBuildError> {
    expression_type_result_with_source_role(source_text, true)
}

fn expression_type_result_with_source_role(
    source_text: &str,
    is_test_file: bool,
) -> Result<ExpressionTypeModel, ExpressionTypeModelBuildError> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");
    let relative_path = if is_test_file {
        "internal/any_interface.test.skiff"
    } else {
        "internal/any_interface.skiff"
    };
    let module_path = if is_test_file {
        "internal.any_interface.__test"
    } else {
        ANY_INTERFACE_MODULE
    };
    let source = CompilerSourceFile::parse(
        PathBuf::from(relative_path),
        module_path.to_string(),
        false,
        is_test_file,
        source_text.to_string(),
        relative_path,
    )
    .expect("test source should parse");
    let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
        .expect("test source should build parsed source facts");
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::new(),
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution should build");
    let expression_sources =
        ExpressionSourceMap::build(&parsed_sources).expect("expression source facts should build");
    let db_metadata = publication_db_metadata_index(
        parsed_sources
            .iter()
            .map(|source| (source.module_path(), source.ast())),
        &BTreeMap::new(),
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("DB metadata should build");
    ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &db_metadata,
        None,
    )
}

fn boxing_source(body: &str) -> String {
    format!(
        r#"
              interface Provider {{
                function name(self: Self) -> string
              }}

              type Host implements Provider {{
                label: string,
              }}

              impl Host {{
                function name() -> string {{ return self.label }}
              }}

              type Other {{
                label: string,
              }}

              {body}
            "#
    )
}

#[test]
fn nullable_union_alias_record_field_matches_nullable_parameter() {
    expression_type_result(
        r#"
              alias Format = "png" | "jpeg" | "webp"
              type Request { format: Format? }

              function consume(format: Format?) -> void {}

              function run(input: Request) -> void {
                consume(input.format)
              }
            "#,
    )
    .expect("nullable union alias field should match the same local parameter type");
}

#[test]
fn nullable_union_alias_does_not_drop_null_from_record_field() {
    let error = expression_type_result(
        r#"
              alias Format = "png" | "jpeg" | "webp"
              type Request { format: Format? }

              function consume(format: Format) -> void {}

              function run(input: Request) -> void {
                consume(input.format)
              }
            "#,
    )
    .expect_err("nullable union alias field must not match a non-null parameter");
    assert!(
        error.message().contains("argument 1 type mismatch"),
        "unexpected diagnostic: {}",
        error.message()
    );
}

#[test]
fn nullable_union_alias_rejects_non_member_literal() {
    let error = expression_type_result(
        r#"
              alias Format = "png" | "jpeg" | "webp"

              function consume(format: Format?) -> void {}

              function run() -> void {
                consume("gif")
              }
            "#,
    )
    .expect_err("non-member literal must not enter a nullable union alias");
    assert!(
        error.message().contains("argument 1 type mismatch"),
        "unexpected diagnostic: {}",
        error.message()
    );
}

#[test]
fn rejects_non_bool_while_condition() {
    let error = expression_type_result(
        r#"
              function run() -> void {
                while 1 {
                  return
                }
              }
            "#,
    )
    .expect_err("while condition must be bool");
    assert!(
        error.message().contains("while condition type mismatch"),
        "unexpected diagnostic: {}",
        error.message()
    );
}

#[test]
fn actor_self_field_assignment_requires_declared_field_type() {
    let error = expression_type_result(
        r#"
              type Counter {
                id: string,
                count: number,
              }

              actor Counter {
                key(id)
                create()
              }

              impl Counter {
                function create() -> void {
                  self.count = 0
                }

                function corrupt() -> void {
                  self.count = "not a number"
                }
              }
            "#,
    )
    .expect_err("Actor self field assignment must be type checked");
    assert!(
        error
            .message()
            .contains("self field assignment type mismatch"),
        "unexpected diagnostic: {}",
        error.message()
    );
}

#[test]
fn explicit_actor_registry_intrinsics_return_nominal_handles() {
    expression_type_result(
        r#"
              type UserActor {
                id: string,
                displayName: string,
                loginCount: number,
              }

              actor UserActor {
                key(id)
                create(displayName: string, loginCount: number)
              }

              impl UserActor {
                function create(self: UserActor, displayName: string, loginCount: number) -> void {
                  self.displayName = displayName
                  self.loginCount = loginCount
                }

                function label() -> string { return self.displayName }
              }

              function load(id: string) -> UserActor {
                const actor: UserActor = std.actor.get<UserActor>(id, "Ada", 1)
                const label: string = actor.label()
                return actor
              }
            "#,
    )
    .expect("actor declarations should be nominal handle types for registry results");
}

#[test]
fn actor_registry_intrinsics_reject_non_actor_wrong_id_and_bootstrap_shape() {
    let error = expression_type_result(
        r#"
              type User { id: string }
              type UserActor { id: string, displayName: string }
              actor UserActor { key(id) create(displayName: string) }

              function invalid() -> void {
                std.actor.get<User>("u1")
                std.actor.get<UserActor>(42, "Ada")
                std.actor.get<UserActor>("u1", 42)
                const actor = std.actor.get<UserActor>("u1", "Ada")
                const leaked = actor.displayName
                const stored = db require UserActor("u1")
              }
            "#,
    )
    .expect_err("invalid actor registry uses must fail");
    let message = error.message();
    assert!(message.contains("is not an actor declaration"), "{message}");
    assert!(message.contains("argument 1"), "{message}");
    assert!(message.contains("argument 2"), "{message}");
    assert!(message.contains("unknown field `displayName`"), "{message}");
    assert!(
        message.contains("cannot be used as a database object"),
        "{message}"
    );
}

#[test]
fn explicit_actor_cannot_be_constructed_as_a_record() {
    let error = expression_type_result(
        r#"
              type UserActor { id: string, displayName: string }
              actor UserActor { key(id) create(displayName: string) }
              function invalid() -> UserActor {
                return UserActor { displayName: "Ada" }
              }
            "#,
    )
    .expect_err("ordinary actor construction must fail");
    assert!(
        error.message().contains("nominal handle")
            && error.message().contains("cannot be constructed directly"),
        "{}",
        error.message()
    );
}

#[test]
fn catch_leaves_accept_nominal_representations_aliases_unions_and_rethrow_envelopes() {
    expression_type_result(
        r#"
              type RecordFailure { message: string }
              type PrimitiveFailure = string
              type GenericFailure<T> { value: T }
              alias TransparentFailure = RecordFailure
              type FailureUnion discriminator "kind" =
                RecordFailure |
                { kind: "synthetic", message: string } |
                "literal"

              function throwEveryShape(
                record: RecordFailure,
                primitive: PrimitiveFailure,
                generic: GenericFailure<string>,
                transparent: TransparentFailure,
                named: FailureUnion,
                anonymous: RecordFailure | PrimitiveFailure
              ) -> void {
                throw record
                throw primitive
                throw generic
                throw transparent
                throw named
                throw anonymous
              }

              function catchEveryShape(value: RecordFailure) -> void {
                const record = catch<RecordFailure>(value)
                const primitive = catch<PrimitiveFailure>(value)
                const generic = catch<GenericFailure<string>>(value)
                const transparent = catch<TransparentFailure>(value)
                const named = catch<FailureUnion>(value)
                const anonymous = catch<RecordFailure | PrimitiveFailure>(value)
              }

              function rethrowStatement(
                exception: Exception<GenericFailure<string>>
              ) -> void {
                rethrow exception
              }

              function rethrowExpression(
                exception: Exception<GenericFailure<string>>
              ) -> GenericFailure<string> {
                return rethrow exception
              }
            "#,
    )
    .expect("nominal catch identities and generic rethrow envelopes should be accepted");
}

#[test]
fn catch_leaves_reject_every_non_nominal_shape_at_throw_and_catch() {
    let cases = [
        ("primitive", "string", ""),
        ("literal", "\"literal\"", ""),
        ("anonymous record", "{ message: string }", ""),
        ("container", "Array<string>", ""),
        (
            "interface",
            "any Marker",
            "interface Marker { function value(self: Self) -> string }",
        ),
        ("unknown", "unknown", ""),
        ("function", "fn(input: string) -> string", ""),
        ("nullable", "RecordFailure?", ""),
        ("unconstrained generic", "T", ""),
        ("mixed union", "RecordFailure | string", ""),
    ];

    for (label, invalid_type, declarations) in cases {
        let type_params = (label == "unconstrained generic")
            .then_some("<T>")
            .unwrap_or("");
        let source = format!(
            r#"
                  type RecordFailure {{ message: string }}
                  {declarations}
                  function invalid{type_params}(
                    value: {invalid_type},
                    valid: RecordFailure
                  ) -> void {{
                    throw value
                    const attempted = catch<{invalid_type}>(valid)
                  }}
                "#,
        );
        let message = match expression_type_result(&source) {
            Ok(_) => panic!("{label} should be rejected"),
            Err(error) => error.message(),
        };
        assert!(
            message.contains("throw payload") && message.contains("invalid catch type"),
            "{label}: {message}"
        );
        assert!(
            message.contains(" at "),
            "{label} diagnostics must retain a source location: {message}"
        );
    }
}

#[test]
fn rethrow_requires_exception_with_valid_non_empty_catch_leaves() {
    let message = expression_type_result(
        r#"
              type Failure { message: string }

              function wrongEnvelope(value: Failure) -> void {
                rethrow value
              }

              function invalidPayload(value: Exception<string>) -> void {
                rethrow value
              }
            "#,
    )
    .expect_err("invalid rethrow operands must fail in source typing")
    .message();

    assert!(
        message.contains("rethrow operand must be Exception<E>"),
        "{message}"
    );
    assert!(
        message.contains("unwrapped primitive or container `string`"),
        "{message}"
    );
    assert!(message.matches(" at ").count() >= 2, "{message}");
}

#[test]
fn package_and_service_test_effect_throw_use_open_nominal_payloads() {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");

    let source_text = r#"
          type ArbitraryFailure { message: string }

          test "open throws" effects {
            dep/tools.run {
              throw: ArbitraryFailure { message: "package" },
            },
            echo/run {
              throw: ArbitraryFailure { message: "service" },
            },
          } {
            assert true
          }
        "#;
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/open_errors.test.skiff"),
        "internal.open_errors.__test".to_string(),
        false,
        true,
        source_text.to_string(),
        "internal/open_errors.test.skiff",
    )
    .expect("test effect source parses");
    let parsed_sources = parse_publication_sources(
        Path::new("/tmp/open-error-test-effects"),
        std::slice::from_ref(&source),
    )
    .expect("test effect source facts build");

    let callable = PackageDependencyCallableAnalysis::new(
        PackageCallableId::new("callable:dep-tools-run"),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    writes_caller_reachable: false,
                    returns_caller_alias: false,
                    throws_caller_alias: false,
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_suspend: false,
                },
            },
            provenance: CallableProvenanceSummary::Unknown {
                reason: CallableProvenanceUnknownReason::AnalysisPending,
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
        may_suspend: false,
    });
    let dependencies = SourceDependencyAnalysisInput::new(
        [(
            "dep".to_string(),
            PackageDependencyAnalysisFacts::new(
                PackageBuildId::new("build:dep"),
                PackageLocalAbiIdentity::new("abi:dep"),
                BTreeMap::from([("tools.run".to_string(), callable)]),
            ),
        )],
        [resolved_contract_fixture(
            "echo",
            "example.echo",
            "run",
            "input",
            "output",
        )],
    )
    .expect("exact dependency analysis facts build");
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::new();

    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: Path::new("/tmp/open-error-test-effects"),
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new("example.com/open-error-test-effects"),
        },
        &dependencies,
    )
    .expect("test-effect throw accepts any nominal payload independent of declared sets");
}

#[test]
fn typed_catch_value_requires_and_respects_tag_narrowing() {
    expression_type_result(
        r#"
              type Payload { value: string }
              type Failure = string

              function make() -> Payload {
                return Payload { value: "ok" }
              }

              function equalBranch() -> Payload? {
                const attempted = catch<Failure>(make())
                if attempted.tag == "ok" { return attempted.value }
                return null
              }

              function reverseComparison() -> Payload? {
                const attempted = catch<Failure>(make())
                if "ok" != attempted.tag { return null }
                return attempted.value
              }

              function earlyReturn() -> Payload? {
                const attempted = catch<Failure>(make())
                if attempted.tag != "ok" { return null }
                return attempted.value
              }

              function nestedCatch() -> Payload? {
                const outer = catch<Failure>(equalBranch())
                if outer.tag != "ok" { return null }
                return outer.value
              }
            "#,
    )
    .expect("ok-tag branches must expose the exact catch success type");

    let unnarrowed = expression_type_result(
        r#"
              type Payload { value: string }
              type Failure = string
              function make() -> Payload { return Payload { value: "ok" } }
              function invalid() -> Payload {
                const attempted = catch<Failure>(make())
                return attempted.value
              }
            "#,
    )
    .expect_err("an un-narrowed catch result must not expose value")
    .message();
    assert!(
        unnarrowed.contains("unknown field `value` on CatchResult"),
        "{unnarrowed}"
    );

    let error_branch = expression_type_result(
        r#"
              type Payload { value: string }
              type Failure = string
              function make() -> Payload { return Payload { value: "ok" } }
              function invalid() -> Payload? {
                const attempted = catch<Failure>(make())
                if attempted.tag == "err" { return attempted.value }
                return null
              }
            "#,
    )
    .expect_err("the error branch must not expose the success value")
    .message();
    assert!(
        error_branch.contains("unknown field `value`"),
        "{error_branch}"
    );
}

#[test]
fn test_assertion_true_flow_narrows_stable_bindings() {
    test_expression_type_result(
        r#"
              type Payload { value: string }
              type Failure = string

              function make() -> Payload {
                return Payload { value: "ok" }
              }

              function maybe() -> Payload? {
                return make()
              }

              test "nullable local" {
                const value: Payload? = maybe()
                assert value != null
                assert value.value == "ok"
              }

              test "tagged catch result" {
                const attempted = catch<Failure>(make())
                assert attempted.tag == "ok"
                assert attempted.value.value == "ok"
              }

              test "conjunction" {
                const value: Payload? = maybe()
                const attempted = catch<Failure>(make())
                assert value != null && attempted.tag == "ok"
                assert value.value == attempted.value.value
              }

              test "nested test block" {
                const value: Payload? = maybe()
                if true {
                  assert value != null
                  assert value.value == "ok"
                }
              }
            "#,
    )
    .expect("assertions in tests must carry their true-flow narrowing forward");
}

#[test]
fn test_assertion_narrowing_fails_closed_for_invalidated_or_unstable_values() {
    let cases = [
        (
            r#"
                  type Payload { value: string }
                  function maybe() -> Payload? { return Payload { value: "ok" } }
                  test "opposite null assertion" {
                    const value: Payload? = maybe()
                    assert value == null
                    assert value.value == "ok"
                  }
                "#,
            "opposite null assertion",
        ),
        (
            r#"
                  type Payload { value: string }
                  function maybe() -> Payload? { return Payload { value: "ok" } }
                  test "unstable call" {
                    assert maybe() != null
                    assert maybe().value == "ok"
                  }
                "#,
            "unstable call expression",
        ),
        (
            r#"
                  type Payload { value: string }
                  function maybe() -> Payload? { return Payload { value: "ok" } }
                  test "reassignment" {
                    let value: Payload? = maybe()
                    assert value != null
                    value = null
                    assert value.value == "ok"
                  }
                "#,
            "reassignment",
        ),
        (
            r#"
                  type Payload { value: string }
                  function maybe() -> Payload? { return Payload { value: "ok" } }
                  test "branch merge" {
                    const value: Payload? = maybe()
                    if true {
                      assert value != null
                    }
                    assert value.value == "ok"
                  }
                "#,
            "branch merge",
        ),
    ];

    for (source, label) in cases {
        let error = test_expression_type_result(source)
            .expect_err("invalid assert narrowing must fail closed")
            .message();
        assert!(
            error.contains("nullable") || error.contains("unknown field"),
            "{label} should retain the unsafe optional type, got:\n{error}"
        );
    }
}

#[test]
fn self_field_resolution_keeps_actor_and_record_owners_distinct() {
    expression_type_result(
        r#"
              type User { name: string }
              type Box<T> { value: T }
              type UserActor { id: string, name: string }
              actor UserActor { key(id) create() }

              impl User {
                function name() -> string { return self.name }
              }
              impl Box<T> {
                function get() -> T { return self.value }
              }
              impl UserActor {
                function create() -> void { self.name = "" }
                function name() -> string { return self.name }
              }
            "#,
    )
    .expect("ordinary, generic, and actor self fields must use their canonical static owner");

    let error = expression_type_result(
        r#"
              type User { name: string }
              type UserActor { id: string, name: string }
              actor UserActor { key(id) create() }

              impl User {
                function invalid() -> string { return self.missing }
              }
              impl UserActor {
                function create() -> void { self.name = "" }
                function invalid() -> string { return self.missing }
              }
            "#,
    )
    .expect_err("unknown ordinary and actor self fields must both fail closed")
    .message();
    assert!(error.contains("unknown field `missing` on User"), "{error}");
    assert!(
        error.contains("unknown field `missing` on UserActor"),
        "{error}"
    );
}

#[test]
fn db_read_projection_publishes_selected_fields_and_automatic_key() {
    expression_type_result(
        r#"
              type Credential {
                id: string,
                label: string,
                apiKey: string,
              }

              db object Credential {
                primary key(id)
                storage apiKey using encrypted
              }

              function projected(id: string) -> { id: string, apiKey: string } {
                const credential = db require Credential(id) {
                  fields { apiKey }
                }
                return { id: credential.id, apiKey: credential.apiKey }
              }
            "#,
    )
    .expect("projected fields and the automatic key should be available to source typing");
}

#[test]
fn db_read_projection_preserves_nested_nullable_and_many_wrappers() {
    expression_type_result(
            r#"
              type Profile {
                displayName: string,
                ignored: number,
              }

              type User {
                id: string,
                profile: Profile?,
              }

              db object User {
                primary key(id)
              }

              function projectedMany() -> Array<{ id: string, profile: { displayName: string }? }> {
                return db find many User {
                  fields { profile.displayName }
                }
              }

              function projectedOptional(id: string) -> { id: string, profile: { displayName: string }? }? {
                return db optional User(id) {
                  fields { profile.displayName }
                }
              }
            "#,
        )
        .expect("nested projected shape should preserve nullable and many wrappers");
}

#[test]
fn db_read_projection_rejects_unknown_duplicate_and_parent_child_paths() {
    let source = |fields: &str| {
        format!(
            r#"
                  type Profile {{ displayName: string }}
                  type User {{ id: string, profile: Profile, label: string }}
                  db object User {{ primary key(id) }}

                  function projected(id: string) -> void {{
                    db require User(id) {{ fields {{ {fields} }} }}
                  }}
                "#
        )
    };

    for (fields, expected) in [
        (
            "missing",
            "db projection references unknown field `missing`",
        ),
        ("label, label", "duplicate db projection field `label`"),
        (
            "profile, profile.displayName",
            "cannot include both `profile` and child path `profile.displayName`",
        ),
    ] {
        let error = expression_type_result(&source(fields))
            .expect_err("invalid projection should fail source typing")
            .message();
        assert!(
            error.contains(expected),
            "projection {fields:?} should report {expected:?}, got:\n{error}"
        );
    }
}

#[test]
fn relational_comparison_accepts_numbers_and_db_string_cursor() {
    expression_type_result(
        r#"
              type Credential { id: string }
              db object Credential { primary key(id) }

              function scan(lastId: string) -> Array<Credential> {
                return db find many Credential {
                  where id > lastId
                  order id asc
                  limit 100
                }
              }

              function numberOrder(left: number, right: number) -> bool {
                return left < right || left <= right || left > right || left >= right
              }

              function lexicalBindingSurvivesDbPredicate(id: number) -> number {
                const count = db count Credential { where id > "credential-0" }
                return id + count
              }
            "#,
    )
    .expect("DB string cursor and numeric relational comparisons should type-check");
}

#[test]
fn relational_comparison_rejects_runtime_strings_mixed_nullable_and_other_types() {
    for (source, label) in [
        (
            r#"
                  function invalid(left: string, right: string) -> bool {
                    return left > right
                  }
                "#,
            "ordinary runtime string relation",
        ),
        (
            r#"
                  function invalid(left: string, right: number) -> bool {
                    return left > right
                  }
                "#,
            "mixed string/number",
        ),
        (
            r#"
                  function invalid(left: string?, right: string) -> bool {
                    return left > right
                  }
                "#,
            "nullable string",
        ),
        (
            r#"
                  function invalid(left: bool, right: bool) -> bool {
                    return left > right
                  }
                "#,
            "non-orderable bool",
        ),
        (
            r#"
                  type Credential { id: string }
                  db object Credential { primary key(id) }

                  function invalid(id: number) -> Array<Credential> {
                    return db find many Credential { where id > id }
                  }
                "#,
            "DB field and shadowed lexical value",
        ),
        (
            r#"
                  type Credential { id: string }
                  db object Credential { primary key(id) }

                  function invalid(id: number?) -> bool {
                    if id != null {
                      const count = db count Credential { where id > id }
                    }
                    return true
                  }
                "#,
            "DB field and non-null-narrowed lexical root",
        ),
        (
            r#"
                  type StoredProfile { name: number }
                  type Credential { id: string, profile: StoredProfile }
                  db object Credential { primary key(id) }

                  type LexicalProfile { name: string? }

                  function invalid(profile: LexicalProfile, lastString: string) -> bool {
                    if profile.name != null {
                      const count = db count Credential { where profile.name > lastString }
                    }
                    return true
                  }
                "#,
            "nested DB field and narrowed lexical path",
        ),
    ] {
        let error = expression_type_result(source)
            .expect_err("invalid relational comparison should fail")
            .message();
        assert!(
            error.contains("binary comparison operand type mismatch"),
            "{label} should report a comparison mismatch, got:\n{error}"
        );
    }
}

#[test]
fn explicit_interface_boxing_and_any_interface_method_call_type_check() {
    expression_type_result(&boxing_source(
        r#"
              function run() -> string {
                const provider: any Provider = Host { label: "host" } as Provider
                return provider.name()
              }
            "#,
    ))
    .expect("explicit boxing and any-interface method call should type-check");
}

#[test]
fn any_interface_internal_named_record_and_function_type_hosts_type_check() {
    expression_type_result(&boxing_source(
        r#"
              type Holder {
                provider: any Provider,
              }

              function consume(handler: fn(input: any Provider) -> any Provider) -> void {}

              function make() -> Holder {
                const holder: Holder = Holder {
                  provider: Host { label: "host" } as Provider,
                }
                return holder
              }
            "#,
    ))
    .expect("internal named record and function type hosts should type-check");
}

#[test]
fn interface_boxing_const_return_publishes_expression_type_fact() {
    let source_text = boxing_source(
        r#"
              const provider: Host = Host { label: "host" }

              function testProvider() -> any Provider {
                return provider as Provider
              }
            "#,
    );
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/any_interface.skiff"),
        ANY_INTERFACE_MODULE.to_string(),
        false,
        false,
        source_text.clone(),
        "internal/any_interface.skiff",
    )
    .expect("test source should parse");
    let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
        .expect("test source should build parsed source facts");
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::new(),
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution should build");
    let expression_sources =
        ExpressionSourceMap::build(&parsed_sources).expect("expression source facts should build");
    let model = ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &PublicationDbMetadataIndex::default(),
        None,
    )
    .expect("interface boxing const return should type-check");
    let key = ExpressionKey::new(
        ANY_INTERFACE_MODULE.to_string(),
        ExpressionOwnerKey::Function("testProvider".to_string()),
        0,
    );
    let fact = model
        .fact(&key)
        .and_then(|fact| fact.ty.as_ref())
        .expect("interface boxing return expression should publish a type fact");
    assert!(matches!(fact.ir, TypeRefIr::AnyInterface { .. }));
}

#[test]
fn concrete_value_does_not_implicitly_box_to_any_interface_parameter() {
    let error = expression_type_result(&boxing_source(
        r#"
              function accepts(provider: any Provider) -> void {}

              function run() -> void {
                accepts(Host { label: "host" })
              }
            "#,
    ))
    .expect_err("concrete value must not implicitly box to any Provider");
    let message = error.message();
    assert!(
        message.contains("argument") && message.contains("any "),
        "unexpected implicit boxing diagnostic: {message}"
    );

    expression_type_result(&boxing_source(
        r#"
              function accepts(provider: any Provider) -> void {}

              function run() -> void {
                accepts(Host { label: "host" } as Provider)
              }
            "#,
    ))
    .expect("explicit boxing should satisfy any Provider parameter");
}

#[test]
fn interface_boxing_rejects_invalid_selector_source_and_conformance() {
    let selector_error = expression_type_result(&boxing_source(
        r#"
              function run() -> void {
                const provider = Host { label: "host" } as string
              }
            "#,
    ))
    .expect_err("as string should fail in expression type checking")
    .message();
    assert!(
        selector_error.contains("interface boxing selector `string`")
            && selector_error.contains("primitive/builtin"),
        "unexpected selector diagnostic: {selector_error}"
    );

    let source_error = expression_type_result(&boxing_source(
        r#"
              function run() -> void {
                const provider = { label: "host" } as Provider
              }
            "#,
    ))
    .expect_err("anonymous record source should not box")
    .message();
    assert!(
        source_error.contains("must be a concrete nominal record"),
        "unexpected source diagnostic: {source_error}"
    );

    let conformance_error = expression_type_result(&boxing_source(
        r#"
              function run() -> void {
                const provider = Other { label: "host" } as Provider
              }
            "#,
    ))
    .expect_err("non-conforming record should not box")
    .message();
    assert!(
        conformance_error.contains("does not explicitly implement interface Provider"),
        "unexpected conformance diagnostic: {conformance_error}"
    );
}

#[test]
fn interface_boxing_rejects_marker_interface() {
    let error = expression_type_result(
        r#"
              interface Marker {}

              type Host implements Marker {
                label: string,
              }

              function run() -> void {
                const provider = Host { label: "host" } as Marker
              }
            "#,
    )
    .expect_err("marker interface should not be object-safe for boxing")
    .message();
    assert!(
        error.contains("not object-safe") && error.contains("marker interface"),
        "unexpected marker diagnostic: {error}"
    );
}

#[test]
fn constructor_validation_error_carries_structured_field_facts() {
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/user.skiff"),
        "internal.user".to_string(),
        false,
        false,
        r#"
              type User {
                name: string,
                email: string,
                age: string,
              }

              function build() -> User {
                return User { name: "Ada", name: "Byron", email: 1, extra: "x" }
              }
            "#
        .to_string(),
        "internal/user.skiff",
    )
    .expect("test source should parse");
    let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
        .expect("test source should build parsed source facts");
    let package_aliases = BTreeMap::new();
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &package_aliases,
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution should build");
    let expression_sources =
        ExpressionSourceMap::build(&parsed_sources).expect("expression source facts should build");

    let error = ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &PublicationDbMetadataIndex::default(),
        None,
    )
    .expect_err("invalid constructor should fail expression type checking");
    let key = ExpressionKey::new(
        "internal.user".to_string(),
        ExpressionOwnerKey::Function("build".to_string()),
        0,
    );
    let validation = error
        .model()
        .constructor_validation(&key)
        .expect("failed constructor should still have structured validation fact");

    assert_eq!(validation.provided_fields.len(), 4);
    assert_eq!(validation.duplicate_fields[0].name, "name");
    assert!(
        validation.duplicate_fields[0].name_span != SourceSpan::synthetic(),
        "duplicate field should retain source name span"
    );
    assert_eq!(validation.unknown_fields[0].name, "extra");
    assert!(
        validation.unknown_fields[0].name_span != SourceSpan::synthetic(),
        "unknown field should retain source name span"
    );
    assert_eq!(validation.missing_required_fields[0].name, "age");
    assert_eq!(validation.type_mismatches[0].name, "email");
    assert_eq!(validation.type_mismatches[0].expected.to_string(), "string");
    assert!(
        validation.type_mismatches[0].value_span != SourceSpan::synthetic(),
        "field mismatch should retain source value span"
    );
}

#[test]
fn db_upsert_result_fields_are_static_expression_type_facts() {
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/db_upsert_result_fields.test.skiff"),
        "internal.db_upsert_result_fields".to_string(),
        false,
        true,
        r#"
              type User {
                id: string,
                name: string,
              }

              db object User {
                name "user"
                primary key(id)
              }

              test "upsert result fields" {
                const r = db upsert User("u1") { name = "Ada" } { name = "Ada" }
                assert r.inserted
                assert r.value.name == "Ada"
              }
            "#
        .to_string(),
        "internal/db_upsert_result_fields.test.skiff",
    )
    .expect("test source should parse");
    let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
        .expect("test source should build parsed source facts");
    let package_aliases = BTreeMap::new();
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &package_aliases,
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution should build");
    let expression_sources =
        ExpressionSourceMap::build(&parsed_sources).expect("expression source facts should build");

    ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &PublicationDbMetadataIndex::default(),
        None,
    )
    .expect("DbUpsertResult.inserted and .value fields should type-check statically");

    let user_ir = TypeRefIr::Record {
        fields: BTreeMap::from([(
            "name".to_string(),
            TypeRefIr::Builtin {
                name: "string".to_string(),
                args: Vec::new(),
            },
        )]),
    };
    let result_ir = TypeRefIr::Builtin {
        name: "DbUpsertResult".to_string(),
        args: vec![user_ir.clone()],
    };
    assert_eq!(
        record_field_type_from_ir(&result_ir, "inserted")
            .expect("inserted field should resolve")
            .ir,
        TypeRefIr::Builtin {
            name: "bool".to_string(),
            args: Vec::new(),
        }
    );
    assert_eq!(
        record_field_type_from_ir(&result_ir, "value")
            .expect("value field should resolve")
            .ir,
        user_ir
    );
}

#[test]
fn runtime_receiver_builtin_calls_publish_static_return_type_facts() {
    let source_text = r#"
              import std

              type RuntimeLiveDoc {
                id: string,
                value: string,
                visits: number,
                rank: number,
              }

              db object RuntimeLiveDoc {
                name "runtime_live_doc"
                primary key(id)
              }

              function run() -> bool {
                const marker = config.require<string>("runtimeLive.db")
                const prefix = "runtime-live-db-".concat(std.crypto.uuidSimple())
                const firstId = prefix.concat("-a")
                const epoch = Date.fromEpochMilliseconds(0)
                const later = epoch.addMilliseconds(5)
                const epochMillis = epoch.toEpochMilliseconds()
                const diffMillis = later.diffMilliseconds(epoch)
                const ordering = epoch.compare(later)
                db insert RuntimeLiveDoc { id = firstId value = marker.concat("-first") visits = 1 rank = 10 }
                return firstId.contains(marker)
              }
            "#;
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/db_receiver_concat.skiff"),
        "internal.db_receiver_concat".to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/db_receiver_concat.skiff",
    )
    .expect("test source should parse");
    let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
        .expect("test source should build parsed source facts");
    let package_aliases = BTreeMap::new();
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &package_aliases,
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution should build");
    let expression_sources =
        ExpressionSourceMap::build(&parsed_sources).expect("expression source facts should build");

    let model = ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &PublicationDbMetadataIndex::default(),
        None,
    )
    .expect("config strings and receiver builtin string calls should type-check statically");
    let owner = ExpressionOwnerKey::Function("run".to_string());

    for (snippet, label, expected) in [
        (
            r#"config.require<string>("runtimeLive.db")"#,
            "config.require<string> result",
            "string",
        ),
        (
            r#""runtime-live-db-".concat(std.crypto.uuidSimple())"#,
            "literal concat result",
            "string",
        ),
        (
            r#"prefix.concat("-a")"#,
            "bound prefix concat result",
            "string",
        ),
        (
            r#"marker.concat("-first")"#,
            "db body marker concat result",
            "string",
        ),
        (
            "epoch.toEpochMilliseconds()",
            "Date.toEpochMilliseconds result",
            "integer",
        ),
        (
            "later.diffMilliseconds(epoch)",
            "Date.diffMilliseconds result",
            "integer",
        ),
        ("epoch.compare(later)", "Date.compare result", "integer"),
        ("firstId.contains(marker)", "contains result", "bool"),
    ] {
        assert_eq!(
            expression_fact_source_text(
                &model,
                &expression_sources,
                source_text,
                "internal.db_receiver_concat",
                &owner,
                snippet,
            ),
            expected,
            "{label} should publish a {expected} expression type fact"
        );
    }
}

#[test]
fn native_signature_local_types_are_externalized_from_the_declaring_module() {
    let production = CompilerSourceFile::parse(
        PathBuf::from("time.skiff"),
        "std.time".to_string(),
        false,
        false,
        r#"
              type Duration = integer
              native function sleep(duration: Duration) -> void
            "#
        .to_string(),
        "time.skiff",
    )
    .expect("production source should parse");
    let test_source = CompilerSourceFile::parse(
        PathBuf::from("time.test.skiff"),
        "std.time.__test".to_string(),
        false,
        true,
        r#"
              import std

              test "duration native signature" {
                const duration = Duration.milliseconds(1)
                std.time.sleep(duration)
              }
            "#
        .to_string(),
        "time.test.skiff",
    )
    .expect("test source should parse");
    let parsed_sources =
        parse_publication_sources(&PathBuf::from("/test"), &[production, test_source])
            .expect("production and test source facts should build");
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::new(),
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution should build");
    let expression_sources =
        ExpressionSourceMap::build(&parsed_sources).expect("expression source facts should build");

    ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &PublicationDbMetadataIndex::default(),
        None,
    )
    .expect("native signature types should retain their declaring module identity");
}

fn expression_fact_source_text(
    model: &ExpressionTypeModel,
    expression_sources: &ExpressionSourceMap,
    source_text: &str,
    module_path: &str,
    owner: &ExpressionOwnerKey,
    snippet: &str,
) -> String {
    expression_sources
        .facts()
        .iter()
        .find_map(|(key, source_fact)| {
            if key.module_path() != module_path || key.owner() != owner {
                return None;
            }
            let span_text = source_text
                .get(source_fact.span.start.offset..source_fact.span.end.offset)?
                .trim();
            if span_text != snippet {
                return None;
            }
            model
                .fact(key)
                .and_then(|fact| fact.ty.as_ref())
                .map(|ty| ty.to_string())
        })
        .unwrap_or_else(|| panic!("expression `{snippet}` should have a type fact"))
}

#[test]
fn single_for_item_wrappers_lock_container_and_local_behavior() {
    let string = TypeRefIr::Builtin {
        name: "string".to_string(),
        args: Vec::new(),
    };
    let number = TypeRefIr::Builtin {
        name: "number".to_string(),
        args: Vec::new(),
    };
    let resolved = |ty: TypeRefIr| ResolvedTypeRef::new(ty);

    // ResolvedTypeRef wrapper: short and std.* full names resolve, Map yields
    // its key type.
    for name in [
        "Array",
        "std.collection.Array",
        "Stream",
        "std.stream.Stream",
    ] {
        let item = single_for_item_type(&resolved(TypeRefIr::Builtin {
            name: name.to_string(),
            args: vec![string.clone()],
        }))
        .expect("single-argument container should resolve");
        assert_eq!(item.ir, string);
    }
    for name in ["Map", "std.collection.Map"] {
        let key = single_for_item_type(&resolved(TypeRefIr::Builtin {
            name: name.to_string(),
            args: vec![string.clone(), number.clone()],
        }))
        .expect("two-argument map should resolve");
        assert_eq!(key.ir, string);
    }
    assert_eq!(
        single_for_item_type(&resolved(TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![string.clone()],
        })),
        None,
        "wrong arity must not resolve"
    );
    assert_eq!(
        single_for_item_type(&resolved(TypeRefIr::Builtin {
            name: "other".to_string(),
            args: Vec::new(),
        })),
        None,
        "non-container must not resolve"
    );

    // PackageTypeRef projection wrapper: Container resolves, including the
    // std.* full names; Local-wrapped containers must stay None.
    let container = |name: &str, arguments: Vec<PackageTypeRef>| PackageTypeRef::Container {
        name: name.to_string(),
        arguments,
    };
    let local = |ty: TypeRefIr| PackageTypeRef::Local { local_type: ty };
    for name in [
        "Array",
        "std.collection.Array",
        "Stream",
        "std.stream.Stream",
    ] {
        assert_eq!(
            single_for_item_projection(&container(
                name,
                vec![PackageTypeRef::Local {
                    local_type: string.clone()
                }]
            )),
            Some(PackageTypeRef::Local {
                local_type: string.clone()
            })
        );
    }
    for name in ["Map", "std.collection.Map"] {
        assert_eq!(
            single_for_item_projection(&container(
                name,
                vec![
                    PackageTypeRef::Local {
                        local_type: string.clone()
                    },
                    PackageTypeRef::Local {
                        local_type: number.clone()
                    },
                ]
            )),
            Some(PackageTypeRef::Local {
                local_type: string.clone()
            })
        );
    }
    assert_eq!(
        single_for_item_projection(&local(TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![string.clone()],
        })),
        None,
        "Local-wrapped container must keep returning None"
    );
    assert_eq!(
        single_for_item_projection(&local(TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![string, number],
        })),
        None,
        "Local-wrapped map must keep returning None"
    );
}

#[test]
fn map_entry_wrappers_lock_full_name_and_local_behavior() {
    let string = TypeRefIr::Builtin {
        name: "string".to_string(),
        args: Vec::new(),
    };
    let number = TypeRefIr::Builtin {
        name: "number".to_string(),
        args: Vec::new(),
    };
    let resolved = |ty: TypeRefIr| ResolvedTypeRef::new(ty);
    let map_ir = |name: &str| TypeRefIr::Builtin {
        name: name.to_string(),
        args: vec![string.clone(), number.clone()],
    };

    // map_entry_types: short name only (pre-existing divergence), Map yields
    // (key, value).
    let (key, value) =
        super::map_entry_types(&resolved(map_ir("Map"))).expect("short-name map should resolve");
    assert_eq!(key.ir, string);
    assert_eq!(value.ir, number);
    assert_eq!(
        super::map_entry_types(&resolved(map_ir("std.collection.Map"))),
        None,
        "map_entry_types must keep rejecting the std.collection.Map full name"
    );
    assert_eq!(
        super::map_entry_types(&resolved(TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![string.clone()],
        })),
        None,
        "wrong arity must not resolve"
    );

    // map_key_type_ir / map_value_type_ir: short and full names both resolve.
    for name in ["Map", "std.collection.Map"] {
        assert_eq!(super::map_key_type_ir(&map_ir(name)), Some(string.clone()));
        assert_eq!(
            super::map_value_type_ir(&map_ir(name)),
            Some(number.clone())
        );
    }
    assert_eq!(super::map_key_type_ir(&map_ir("other")), None);

    // map_entry_projections: Container (short and full names) resolves;
    // Local-wrapped map stays None.
    let container = |name: &str, arguments: Vec<PackageTypeRef>| PackageTypeRef::Container {
        name: name.to_string(),
        arguments,
    };
    for name in ["Map", "std.collection.Map"] {
        let entry = super::map_entry_projections(&container(
            name,
            vec![
                PackageTypeRef::Local {
                    local_type: string.clone(),
                },
                PackageTypeRef::Local {
                    local_type: number.clone(),
                },
            ],
        ))
        .expect("container map should resolve");
        assert_eq!(
            entry,
            (
                PackageTypeRef::Local {
                    local_type: string.clone()
                },
                PackageTypeRef::Local {
                    local_type: number.clone()
                },
            )
        );
    }
    assert_eq!(
        super::map_entry_projections(&PackageTypeRef::Local {
            local_type: map_ir("Map"),
        }),
        None,
        "Local-wrapped map must keep returning None"
    );
}

#[test]
fn package_type_ref_ir_wrapper_preserves_local_internal_schema_rewrite() {
    let schema_ir = TypeRefIr::PackageSchema {
        package_id: "example.types".to_string(),
        stable_schema_key: "Payload".to_string(),
        package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new("type:payload"),
    };
    let package_symbol = TypeRefIr::PackageSymbol {
        symbol: skiff_artifact_model::PackageSymbolRef {
            package: skiff_artifact_model::PackageRefIr::PackageId {
                package_id: "example.types".to_string(),
            },
            symbol_path: "Payload".to_string(),
            abi_expectation: None,
        },
    };
    let local = |ty: TypeRefIr| PackageTypeRef::Local { local_type: ty };

    // Top-level Local keeps the historical rewrite: PackageSchema -> PackageSymbol.
    assert_eq!(
        super::package_type_ref_ir(&local(schema_ir.clone())),
        package_symbol
    );

    // Nested Local inside a Container is rewritten too; core folded alone keeps
    // the Local subtree verbatim, so the wrapper must differ from it here.
    let nested = PackageTypeRef::Container {
        name: "Array".to_string(),
        arguments: vec![local(schema_ir.clone())],
    };
    assert_eq!(
        super::package_type_ref_ir(&nested),
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![package_symbol.clone()],
        }
    );
    assert_ne!(
        super::package_type_ref_ir(&nested),
        skiff_compiler_core::type_ref::package_type_ref_to_ir(&nested)
    );

    // Non-Local PackageSchema folds identically to core folded.
    let direct_schema = PackageTypeRef::PackageSchema {
        package_id: "example.types".to_string(),
        stable_schema_key: "Payload".to_string(),
        package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new("type:payload"),
    };
    assert_eq!(
        super::package_type_ref_ir(&direct_schema),
        skiff_compiler_core::type_ref::package_type_ref_to_ir(&direct_schema)
    );

    // Local without PackageSchema stays verbatim.
    assert_eq!(
        super::package_type_ref_ir(&local(TypeRefIr::builtin("string"))),
        TypeRefIr::builtin("string")
    );
}

#[test]
fn package_type_ref_ir_rewrites_identity_inside_local_any_interface() {
    let any = PackageTypeRef::AnyInterface {
        interface: Box::new(PackageTypeRef::Local {
            local_type: TypeRefIr::PackageSchema {
                package_id: "example.interfaces".to_string(),
                stable_schema_key: "Reader".to_string(),
                package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new(
                    "type:reader",
                ),
            },
        }),
        arguments: Vec::new(),
    };
    let wrapper_ir = super::package_type_ref_ir(&any);
    let folded_ir = skiff_compiler_core::type_ref::package_type_ref_to_ir(&any);
    assert_ne!(
        wrapper_ir, folded_ir,
        "Local interface identity must be rewritten by the etm wrapper"
    );
    let TypeRefIr::AnyInterface { interface } = &wrapper_ir else {
        panic!("expected AnyInterface");
    };
    assert!(matches!(
        serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).unwrap(),
        TypeRefIr::PackageSymbol { .. }
    ));
}

#[test]
fn ternary_accepts_matching_and_widening_branch_types() {
    expression_type_result(
        r#"
              type User { name: string }

              function pick(
                flag: bool,
                a: string,
                b: string,
                count: integer,
                ratio: number,
                user: User?
              ) -> string {
                const same = flag ? a : b
                const widened = flag ? count : ratio
                const literalWidened = flag ? "a" : "b"
                const nullable = user != null ? user.name : null
                const neverBranch = flag ? throw User { name: "boom" } : b
                return same
              }
            "#,
    )
    .expect("compatible ternary branches must type check");
}

#[test]
fn ternary_rejects_incompatible_branch_types() {
    let error = expression_type_result(
        r#"
              function pick(flag: bool, a: string, b: number) -> string {
                const value = flag ? a : b
                return value
              }
            "#,
    )
    .expect_err("incompatible ternary branches must fail");
    assert!(
        error
            .message()
            .contains("ternary branches have incompatible types")
            && error.message().contains("string")
            && error.message().contains("number"),
        "{}",
        error.message()
    );
}

#[test]
fn ternary_requires_bool_condition() {
    let error = expression_type_result(
        r#"
              function pick(a: string, b: string) -> string {
                const value = a ? b : a
                return value
              }
            "#,
    )
    .expect_err("ternary condition must be bool");
    assert!(
        error.message().contains("ternary condition type mismatch"),
        "{}",
        error.message()
    );
}

#[test]
fn ternary_accepts_non_nullable_annotated_result() {
    expression_type_result(
        r#"
              function pick(flag: bool, a: string, b: string) -> string {
                const value: string = flag ? a : b
                return value
              }
            "#,
    )
    .expect("joined string branches must satisfy a string annotation");
}

#[test]
fn ternary_null_branch_result_is_assignable_to_nullable_annotation() {
    expression_type_result(
        r#"
              function pick(flag: bool, value: string) -> string? {
                const result: string? = flag ? value : null
                return result
              }
            "#,
    )
    .expect("null branch must join to a nullable result");
}
