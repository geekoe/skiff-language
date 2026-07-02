use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use skiff_compiler::{
    build_service_publication, collect_source_tree, read_service_config,
    test_support::project_fixtures::{
        write_package_api_yml, write_package_manifest, write_package_source,
    },
    PackageResolutionDirs, PublicationId, ServicePublicationBuildInput,
};
mod common;
use common::artifacts::{
    assert_publish_error_contains_without_package_dirs as assert_publish_error_contains,
    build_temp_service_publication, publish_error_message_without_package_dirs,
};
use common::TestDir;

#[test]
fn service_implementations_conform_rejects_legacy_mongo_provider_package() {
    let temp = write_legacy_mongo_service_project();
    let root = temp.path();
    let config = read_service_config(&root).unwrap();
    let source_tree = collect_source_tree(&root).unwrap();

    let package_dirs = PackageResolutionDirs {
        package_dirs: vec![root.join(".skiff-packages")],
    };
    let error = build_service_publication(ServicePublicationBuildInput {
        config: &config,
        source_tree: &source_tree,
        package_dirs,
        ..ServicePublicationBuildInput::new(&config, &source_tree)
    })
    .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains(".skiff-packages/skiff~run~~mongo/1.0.0/mongo.skiff"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("legacy provider syntax has been removed"),
        "unexpected error: {message}"
    );
}

#[test]
fn service_api_conformance_accepts_exported_impl_methods() {
    let temp = write_example_api_handler_project(
        "pass",
        example_service_impl(
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
        ),
    );
    let published = build_temp_service_publication(temp.path());

    assert_eq!(
        published.artifacts.service_assembly.value["operations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn implemented_interface_signature_mismatch_fails_projection_conformance() {
    let temp = write_api_service_project(
        "implemented-interface-signature-mismatch",
        r#"
            type Input {}
            type Output {}
            interface ExampleApi {
              function save(input: Input) -> Output
            }
        "#,
        r#"
            type ExampleService implements root.api.example.ExampleApi {}
            impl ExampleService {
              function save(self: ExampleService, input: root.api.example.Output) -> root.api.example.Output {
                return {}
              }
            }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "type internal.example.ExampleService method save signature does not match interface api.example.ExampleApi",
            "expected params [\"api.example.Input\"]",
            "got params [\"api.example.Output\"]",
        ],
    );
}

#[test]
fn may_suspend_metadata_is_copied_to_service_operations() {
    let sleep_argument = time_sleep_argument_source();
    let methods = format!(
        r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {{
    return {{}}
  }}

  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {{
    std.time.sleep({sleep_argument})
    return Array.empty<Output>()
  }}
"#,
    );
    let temp =
        write_example_api_handler_project("operation-may-suspend", example_service_impl(&methods));

    let published = build_temp_service_publication(temp.path());

    assert_eq!(
        service_operation(&published, "ExampleService.save")["maySuspend"],
        false
    );
    assert_eq!(
        service_operation(&published, "ExampleService.list")["maySuspend"],
        true
    );
}

fn time_sleep_argument_source() -> &'static str {
    let source =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../std/time.skiff"))
            .unwrap_or_default();
    if source.contains("sleep(duration: Duration)") {
        "Duration.milliseconds(1)"
    } else {
        "1"
    }
}

#[test]
fn may_suspend_metadata_treats_qualified_external_receiver_as_unknown() {
    let temp = write_example_api_handler_project(
        "may-suspend-qualified-receiver",
        r#"
            type Counter {}

            impl Counter {
              function next(self: Counter, input: number) -> number {
                return input + 1
              }
            }

            function externalCounterCall(counter: root.internal.other.Counter) -> number {
              return counter.next(1)
            }

            type ExampleService {}

            impl ExampleService {
            function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
              return {}
            }
            function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
              return Array.empty<root.api.example.Output>()
            }
            }
"#,
    );
    fs::write(
        temp.path().join("internal").join("other.skiff"),
        r#"
            type Counter {}
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let artifact = file_artifact_value(&published, "internal/example.skiff");

    assert_eq!(
        code_function(&artifact, "externalCounterCall")["maySuspend"],
        true
    );
}

#[test]
fn re_exported_type_publishes_impl_methods() {
    let temp = write_example_api_handler_project(
        "re-exported-type-impl-methods",
        r#"
            type ExampleService {}

            impl ExampleService {
              function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
                return {}
              }
            }
"#,
    );

    let published = build_temp_service_publication(temp.path());
    let save = service_operation(&published, "ExampleService.save");

    assert_eq!(save["parameters"][0]["type"]["name"], "api.example.Input");
    assert_eq!(save["returnType"]["name"], "api.example.Output");
}

#[test]
fn private_return_type_projects_as_closure_only_schema() {
    let temp = write_example_api_handler_project(
        "private-return-type-closure",
        r#"
            type OtherOutput {}
            type ExampleService {}

            impl ExampleService {
            function save(self: ExampleService, input: root.api.example.Input) -> OtherOutput {
              return {}
            }
            function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
              return Array.empty<root.api.example.Output>()
            }
            }
"#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = service_operation(&published, "ExampleService.save");
    let schema = &published.artifacts.contract_schema.value["schema"];

    assert_eq!(
        operation["returnType"]["name"],
        "internal.example.OtherOutput"
    );
    assert_eq!(
        operation["response"]["xSkiffSymbol"],
        "internal.example.OtherOutput"
    );
    assert_closure_only_contract_type(schema, "internal.example.OtherOutput");
}

#[test]
fn implementation_parameter_names_are_published_as_protocol_surface() {
    let temp = write_example_api_handler_project(
        "param-name",
        example_service_impl(
            r#"
  function save(self: ExampleService, value: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
        ),
    );

    let published = build_temp_service_publication(temp.path());
    assert_eq!(
        service_operation(&published, "ExampleService.save")["parameters"][0]["name"],
        "value"
    );
}

#[test]
fn zero_parameter_implementation_operation_publishes() {
    let temp = write_example_api_handler_project(
        "param-count",
        example_service_impl(
            r#"
  function save(self: ExampleService) -> root.api.example.Output {
    return {}
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
        ),
    );

    let published = build_temp_service_publication(temp.path());
    assert_eq!(
        service_operation(&published, "ExampleService.save")["parameters"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn private_parameter_type_projects_as_closure_only_schema() {
    let temp = write_example_api_handler_project(
        "private-param-type-closure",
        r#"
            type OtherInput {}
            type ExampleService {}

            impl ExampleService {
            function save(self: ExampleService, input: OtherInput) -> root.api.example.Output {
              return {}
            }
            function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
              return Array.empty<root.api.example.Output>()
            }
            }
"#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = service_operation(&published, "ExampleService.save");
    let schema = &published.artifacts.contract_schema.value["schema"];

    assert_eq!(
        operation["parameters"][0]["type"]["name"],
        "internal.example.OtherInput"
    );
    assert_eq!(
        operation["parameters"][0]["schema"]["xSkiffSymbol"],
        "internal.example.OtherInput"
    );
    assert_closure_only_contract_type(schema, "internal.example.OtherInput");
}

#[test]
fn missing_handler_module_fails_publish() {
    let temp = TestDir::new("skiff-service-conformance", "missing-handler-module");
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
internal:
  missing:
    ExampleService: internal.missing.ExampleService
api:
  example:
    Input: api.example.Input
    Output: api.example.Output
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("example.skiff"),
        default_api_source(),
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("example.skiff"),
        default_implementation_source(),
    )
    .unwrap();

    assert_publish_error_contains(
        temp.path(),
        &["internal.missing.ExampleService", "not found"],
    );
}

#[test]
fn service_api_accepts_source_types_projected_to_public_alias() {
    let temp = write_api_service_project(
        "api-handler-source-type-projection",
        r#"
            type Input {}
            type Output {}
        "#,
        r#"
            type SourceService {}

            impl SourceService {
            function save(self: SourceService, input: root.api.example.Input) -> root.api.example.Output {
              return {}
            }
            }
"#,
    );

    build_temp_service_publication(temp.path());
}

#[test]
fn service_api_projects_private_same_name_signature_type_as_closure_only() {
    let temp = write_api_service_project(
        "api-handler-private-same-name-closure",
        r#"
            type Input {}
            type Output {}
        "#,
        r#"
            type Input {}
            type SourceService {}

            impl SourceService {
            function save(self: SourceService, input: Input) -> root.api.example.Output {
              return {}
            }
            }
"#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = service_operation(&published, "SourceService.save");
    let schema = &published.artifacts.contract_schema.value["schema"];

    assert_eq!(
        operation["parameters"][0]["type"]["name"],
        "internal.example.Input"
    );
    assert_eq!(
        operation["parameters"][0]["schema"]["xSkiffSymbol"],
        "internal.example.Input"
    );
    assert!(schema["types"].get("api.example.Input").is_some());
    assert_closure_only_contract_type(schema, "internal.example.Input");
}

#[test]
fn exported_service_types_are_keyed_by_public_path() {
    let temp = TestDir::new("skiff-service-conformance", "namespaced-duplicate-types");
    fs::create_dir_all(temp.path().join("public")).unwrap();
    fs::create_dir_all(temp.path().join("admin")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
UserQuery: internal.user.UserQuery
public:
  Input: public.Input
admin:
  Input: internal.admin.Input
  AdminQuery: internal.admin.AdminQuery
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("public.skiff"),
        r#"
type Input { value: string }
"#,
    )
    .unwrap();
    fs::write(temp.path().join("admin.skiff"), "").unwrap();
    fs::write(
        temp.path().join("internal").join("user.skiff"),
        r#"
type UserQuery {}
impl UserQuery {
  function get(self: UserQuery, input: public.Input) -> public.Input {
    return input
  }
}
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("admin.skiff"),
        r#"
type Input { value: string }
type AdminQuery {}
impl AdminQuery {
  function get(self: AdminQuery, input: admin.Input) -> admin.Input {
    return input
  }
}
"#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let types = &published.artifacts.contract_schema.value["schema"]["types"];
    assert!(types.get("public.Input").is_some());
    assert!(types.get("admin.Input").is_some());
    assert_eq!(
        published.artifacts.contract_schema.value["schema"]["interfaces"]["UserQuery"]
            ["operations"]["get"]["params"][0]["type"]["name"],
        "public.Input"
    );
    assert_eq!(
        published.artifacts.contract_schema.value["schema"]["interfaces"]["admin.AdminQuery"]
            ["operations"]["get"]["params"][0]["type"]["name"],
        "admin.Input"
    );
}

#[test]
fn service_export_cross_module_root_type_ref_uses_public_path_in_canonical_schema() {
    let temp = TestDir::new("skiff-service-conformance", "cross-export-root-type-ref");
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
user:
  UserService: internal.user.UserService
api:
  group:
    Group: api.group.Group
  user:
    User: api.user.User
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("group.skiff"),
        r#"
type Group {
  id: string
}
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("user.skiff"),
        r#"
type User {
  group: root.api.group.Group
}
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("user.skiff"),
        r#"
type UserService {}
impl UserService {
  function get(self: UserService, user: root.api.user.User) -> root.api.group.Group {
    return root.api.group.Group { id: "" }
  }
}
"#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let schema = &published.artifacts.contract_schema.value["schema"];

    assert_eq!(
        schema["types"]["api.user.User"]["fields"]["group"]["name"],
        "api.group.Group"
    );
    assert_eq!(
        schema["interfaces"]["user.UserService"]["operations"]["get"]["params"][0]["type"]["name"],
        "api.user.User"
    );
    assert_eq!(
        schema["interfaces"]["user.UserService"]["operations"]["get"]["returnType"]["name"],
        "api.group.Group"
    );
}

#[test]
fn date_from_epoch_requires_integer_argument() {
    let temp = write_example_api_handler_project(
        "operation-date-from-epoch-integer-only",
        example_service_impl(
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }

  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    const createdAt = Date.fromEpochMilliseconds("1")
    return Array.empty<Output>()
  }
"#,
        ),
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "call `Date.fromEpochMilliseconds` argument 1 type mismatch",
            "expected integer",
            "found \"1\"",
        ],
    );
}

#[test]
fn date_root_calls_and_boundary_types_publish_as_date_time_schema() {
    let temp = write_api_handler_project(
        "date-root-boundary-schema",
        r#"
            type DateEnvelope {
              createdAt: Date,
              maybeAt: Date?,
              timeline: Array<Date>
            }
        "#,
        r#"
            type ExampleService {}

            impl ExampleService {
              function save(self: ExampleService, input: root.api.example.DateEnvelope) -> root.api.example.DateEnvelope {
                const now = Date.now()
                const epoch = Date.fromEpochMilliseconds(0)
                const parsed = Date.requireParse("1970-01-01T00:00:00.000Z")
                if now.isBefore(epoch) {
                  return input
                }
                if parsed.isBefore(epoch) {
                  return input
                }
                return input
              }
            }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == "ExampleService.save")
        .expect("save operation should be published");
    let envelope_schema = &operation["parameters"][0]["schema"];
    let fields = &envelope_schema["properties"];

    assert_eq!(fields["createdAt"]["type"], "string");
    assert_eq!(fields["createdAt"]["format"], "date-time");
    assert_eq!(fields["createdAt"]["xSkiffSymbol"], "Date");
    assert_eq!(fields["maybeAt"]["nullable"], true);
    assert_eq!(fields["maybeAt"]["format"], "date-time");
    assert_eq!(fields["timeline"]["type"], "array");
    assert_eq!(fields["timeline"]["items"]["format"], "date-time");
    assert_eq!(fields["timeline"]["items"]["xSkiffSymbol"], "Date");
}

#[test]
fn exported_alias_name_does_not_change_contract_schema_identity() {
    let first = write_api_handler_project(
        "alias-identity-first",
        r#"
            type User { id: string }
            alias UserList = Array<User>
        "#,
        example_service_impl(
            r#"
  function save(self: ExampleService, input: root.api.example.UserList) -> root.api.example.UserList {
    return input
  }
  function list(self: ExampleService, limit: number) -> root.api.example.UserList {
    return Array.empty<root.api.example.User>()
  }
"#,
        ),
    );
    let second = write_api_handler_project(
        "alias-identity-second",
        r#"
            type User { id: string }
            alias Members = Array<User>
        "#,
        example_service_impl(
            r#"
  function save(self: ExampleService, input: root.api.example.Members) -> root.api.example.Members {
    return input
  }
  function list(self: ExampleService, limit: number) -> root.api.example.Members {
    return Array.empty<root.api.example.User>()
  }
"#,
        ),
    );

    let first = build_temp_service_publication(first.path());
    let second = build_temp_service_publication(second.path());

    assert_eq!(
        first.manifest.service.protocol_identity,
        second.manifest.service.protocol_identity
    );
    assert_eq!(
        first.artifacts.contract_schema.value["schema"],
        second.artifacts.contract_schema.value["schema"]
    );
    let operation = first
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "ExampleService.save")
        .unwrap();
    assert_eq!(operation.parameters[0].schema.schema_type(), Some("array"));
}

#[test]
fn exported_alias_cycle_is_rejected_in_contract() {
    let temp = write_api_handler_project(
        "alias-cycle-contract",
        r#"
            alias A = B
            alias B = A
        "#,
        example_service_impl(
            r#"
  function save(self: ExampleService, input: root.api.example.A) -> root.api.example.B {
    return input
  }
  function list(self: ExampleService, limit: number) -> root.api.example.B {
    return {}
  }
"#,
        ),
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "recursive alias cycle",
            "api.example.A",
            "api.example.B",
            "api.example.A",
        ],
    );
}

#[test]
fn exported_alias_is_not_a_contract_representation_type() {
    let temp = write_api_handler_project(
        "alias-schema-closure",
        r#"
            type User { id: string }
            alias Users = Array<User>
            alias UserList = Users
        "#,
        example_service_impl(
            r#"
  function save(self: ExampleService, input: root.api.example.UserList) -> root.api.example.UserList {
    return input
  }
  function list(self: ExampleService, limit: number) -> root.api.example.UserList {
    return Array.empty<root.api.example.User>()
  }
"#,
        ),
    );

    let published = build_temp_service_publication(temp.path());
    let schema = &published.artifacts.contract_schema.value["schema"];
    assert!(schema["types"].get("api.example.User").is_some());
    assert!(schema["types"].get("api.example.UserList").is_none());

    let operation = published.artifacts.service_assembly.value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == "ExampleService.save")
        .unwrap();
    assert_eq!(operation["parameters"][0]["schema"]["type"], "array");
    assert!(operation["parameters"][0]["schema"]
        .get("xSkiffSymbol")
        .is_none());
    assert_eq!(operation["parameters"][0]["type"]["kind"], "builtin");
    assert_eq!(operation["parameters"][0]["type"]["name"], "Array");
    assert_eq!(
        operation["parameters"][0]["type"]["args"][0]["fields"]["id"]["name"],
        "string"
    );
    assert_eq!(operation["returnType"]["kind"], "builtin");
    assert_eq!(operation["returnType"]["name"], "Array");
    assert_eq!(
        operation["returnType"]["args"][0]["fields"]["id"]["name"],
        "string"
    );
}

#[test]
fn same_bare_alias_name_expands_in_each_source_scope() {
    let temp = write_example_api_handler_project(
        "alias-same-name-source-scope",
        default_implementation_source(),
    );
    fs::write(
        temp.path().join("internal").join("a.skiff"),
        r#"
            alias Id = string
            function useA(id: Id) -> Id {
              return id
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("b.skiff"),
        r#"
            alias Id = number
            function useB(id: Id) -> Id {
              return id
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let a = file_artifact_value(&published, "internal/a.skiff");
    let b = file_artifact_value(&published, "internal/b.skiff");

    let use_a = declaration_function(&a, "useA");
    assert_type_ref_resolves_to_builtin(&a, &use_a["params"][0]["ty"], "string");
    assert_type_ref_resolves_to_builtin(&a, &use_a["returnType"], "string");

    let use_b = declaration_function(&b, "useB");
    assert_type_ref_resolves_to_builtin(&b, &use_b["params"][0]["ty"], "number");
    assert_type_ref_resolves_to_builtin(&b, &use_b["returnType"], "number");
}

#[test]
fn bare_alias_from_other_source_does_not_expand_without_local_alias() {
    let temp = write_example_api_handler_project(
        "alias-no-cross-source-bare-capture",
        default_implementation_source(),
    );
    fs::write(
        temp.path().join("internal").join("alias_owner.skiff"),
        r#"
            alias Id = string
            function owner(id: Id) -> Id {
              return id
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("no_alias.skiff"),
        r#"
            type Id {
              value: number,
            }

            function keepNominal(id: Id) -> Id {
              return id
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let alias_owner_artifact = file_artifact_value(&published, "internal/alias_owner.skiff");
    let owner = declaration_function(&alias_owner_artifact, "owner");
    assert_type_ref_resolves_to_builtin(&alias_owner_artifact, &owner["params"][0]["ty"], "string");

    let no_alias_artifact = file_artifact_value(&published, "internal/no_alias.skiff");
    let no_alias = declaration_function(&no_alias_artifact, "keepNominal");
    assert_type_ref_is_local_record_with_field(
        &no_alias_artifact,
        &no_alias["params"][0]["ty"],
        "Id",
        "value",
        "number",
    );
    assert_type_ref_is_local_record_with_field(
        &no_alias_artifact,
        &no_alias["returnType"],
        "Id",
        "value",
        "number",
    );
}

#[test]
fn qualified_cross_module_alias_cycle_is_rejected_without_bare_name_false_cycle() {
    let temp = write_example_api_handler_project(
        "alias-qualified-cross-module-cycle",
        default_implementation_source(),
    );
    fs::write(
        temp.path().join("internal").join("a.skiff"),
        r#"
            alias A = internal.b.B
            alias Id = string
            function useA(id: Id) -> Id {
              return id
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("b.skiff"),
        r#"
            alias B = internal.a.A
            alias Id = number
            function useB(id: Id) -> Id {
              return id
            }
        "#,
    )
    .unwrap();

    assert_publish_error_contains(
        temp.path(),
        &[
            "recursive alias cycle",
            "internal.a.A",
            "internal.b.B",
            "internal.a.A",
        ],
    );
}

#[test]
fn contract_private_alias_scope_is_per_exported_source() {
    let temp = TestDir::new("skiff-service-conformance", "contract-private-alias-scope");
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
alpha:
  AlphaService: internal.alpha.AlphaService
api:
  alpha:
    AlphaPayload: api.alpha.AlphaPayload
  beta:
    BetaPayload: api.beta.BetaPayload
beta:
  BetaService: internal.beta.BetaService
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("alpha.skiff"),
        r#"
            type AlphaPayload { a: string }
            alias Payload = AlphaPayload
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("beta.skiff"),
        r#"
            type BetaPayload { b: number }
            alias Payload = BetaPayload
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("alpha.skiff"),
        r#"
            type AlphaService {}

            impl AlphaService {
            function alpha(self: AlphaService, input: root.api.alpha.AlphaPayload) -> root.api.alpha.AlphaPayload {
              return input
            }
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("beta.skiff"),
        r#"
            type BetaService {}

            impl BetaService {
            function beta(self: BetaService, input: root.api.beta.BetaPayload) -> root.api.beta.BetaPayload {
              return input
            }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let alpha = service_operation(&published, "alpha.AlphaService.alpha");
    assert_eq!(
        alpha["parameters"][0]["type"]["name"],
        "api.alpha.AlphaPayload"
    );
    assert_eq!(
        alpha["parameters"][0]["type"]["fields"]["a"]["name"],
        "string"
    );
    assert_eq!(alpha["returnType"]["name"], "api.alpha.AlphaPayload");

    let beta = service_operation(&published, "beta.BetaService.beta");
    assert_eq!(
        beta["parameters"][0]["type"]["name"],
        "api.beta.BetaPayload"
    );
    assert_eq!(
        beta["parameters"][0]["type"]["fields"]["b"]["name"],
        "number"
    );
    assert_eq!(beta["returnType"]["name"], "api.beta.BetaPayload");
}

#[test]
fn contract_bare_alias_from_other_exported_source_does_not_capture_local_type() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "contract-no-cross-source-bare-alias-capture",
    );
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
gamma:
  GammaService: internal.example.GammaService
api:
  gamma:
    Payload: api.gamma.Payload
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("owner.skiff"),
        r#"
            alias Payload = string
            type OwnerMarker {}
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("gamma.skiff"),
        r#"
            type Payload { value: number }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("example.skiff"),
        r#"
            type GammaService {}
            impl GammaService {
            function gamma(self: GammaService, input: root.api.gamma.Payload) -> root.api.gamma.Payload {
              return input
            }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let gamma = service_operation(&published, "gamma.GammaService.gamma");
    assert_eq!(gamma["parameters"][0]["type"]["name"], "api.gamma.Payload");
    assert_eq!(
        gamma["parameters"][0]["type"]["fields"]["value"]["name"],
        "number"
    );
    assert_eq!(gamma["returnType"]["name"], "api.gamma.Payload");
}

#[test]
fn contract_public_qualified_alias_cycle_is_rejected() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "contract-public-qualified-alias-cycle",
    );
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api.yml"),
        r#"
a:
  AService: internal.a.AService
api:
  a:
    A: api.a.A
  b:
    B: api.b.B
b:
  BService: internal.b.BService
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("a.skiff"),
        r#"
            alias A = api.b.B
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("b.skiff"),
        r#"
            alias B = api.a.A
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("a.skiff"),
        r#"
            type AService {}

            impl AService {
              function a(self: AService, input: root.api.a.A) -> root.api.a.A {
                return input
              }
            }
        "#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("b.skiff"),
        r#"
            type BService {}

            impl BService {
              function b(self: BService, input: root.api.b.B) -> root.api.b.B {
                return input
              }
            }
        "#,
    )
    .unwrap();

    assert_publish_error_contains(
        temp.path(),
        &["recursive alias cycle", "api.a.A", "api.b.B", "api.a.A"],
    );
}

#[test]
fn implementation_body_changes_revision_without_protocol_identity_change() {
    let first = write_example_api_handler_project(
        "identity-first",
        example_service_impl(
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
        ),
    );
    let second = write_example_api_handler_project(
        "identity-second",
        example_service_impl(
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    const value = input
    return {}
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
        ),
    );

    let first_published = build_temp_service_publication(first.path());
    let second_published = build_temp_service_publication(second.path());

    assert_eq!(
        first_published.manifest.service.protocol_identity,
        second_published.manifest.service.protocol_identity
    );
    assert_ne!(
        first_published.manifest.service.revision_id,
        second_published.manifest.service.revision_id
    );
}

#[test]
fn contract_comments_and_whitespace_do_not_change_protocol_identity() {
    let implementation_source = example_service_impl(
        r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return root.api.example.Output { result: "" }
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
    );
    let first = write_api_handler_project(
        "identity-contract-clean",
        r#"
            type Input { value: string }
            type Output { result: string }
        "#,
        implementation_source.as_str(),
    );
    let second = write_api_handler_project(
        "identity-contract-commented",
        r#"
            // Contract comments must not affect protocol identity.
            type Input {
              value: string,
            }

            /* Formatting changes must not affect protocol identity. */
            type Output {
              result: string,
            }

        "#,
        implementation_source.as_str(),
    );

    let first_published = build_temp_service_publication(first.path());
    let second_published = build_temp_service_publication(second.path());

    assert_eq!(
        first_published.manifest.service.protocol_identity,
        second_published.manifest.service.protocol_identity
    );
}

#[test]
fn exported_type_shape_change_changes_protocol_identity() {
    let implementation_source = example_service_impl(
        r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return root.api.example.Output { result: "" }
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
    );
    let first = write_api_handler_project(
        "identity-contract-shape-first",
        r#"
            type Input { value: string }
            type Output { result: string }
        "#,
        implementation_source.as_str(),
    );
    let second = write_api_handler_project(
        "identity-contract-shape-second",
        r#"
            type Input { value: string, traceId: string }
            type Output { result: string }
        "#,
        implementation_source.as_str(),
    );

    let first_published = build_temp_service_publication(first.path());
    let second_published = build_temp_service_publication(second.path());

    assert_ne!(
        first_published.manifest.service.protocol_identity,
        second_published.manifest.service.protocol_identity
    );
}

#[test]
fn unknown_contract_boundary_type_fails_publish() {
    let temp = write_api_handler_project(
        "unknown-contract-boundary-type",
        r#"
            type Output { result: string }
        "#,
        example_service_impl(
            r#"
  function save(self: ExampleService, input: MissingInput) -> root.api.example.Output {
    return root.api.example.Output { result: "" }
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
        ),
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "unresolved type `MissingInput`",
        ],
    );
}

#[test]
fn duplicate_service_api_implementation_type_fails_publish() {
    let temp = write_api_handler_project(
        "duplicate-contract-interface",
        r#"
            type Input {}
            type Output {}
        "#,
        r#"
type ExampleService {}
type ExampleService {}

impl ExampleService {
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
}
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "duplicate publication api schema type ExampleService",
        ],
    );
}

#[test]
fn duplicate_exported_contract_operation_fails_publish() {
    let temp = write_api_handler_project(
        "duplicate-contract-operation",
        r#"
            type Input {}
            type Output {}
        "#,
        r#"
            type ExampleService {}

            impl ExampleService {
            function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
              return {}
            }
            function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
              return input
            }
            }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "duplicate publication api callable ExampleService.save",
        ],
    );
}

#[test]
fn anonymous_record_boundary_type_fails_publish() {
    let temp = write_api_handler_project(
        "anonymous-record-boundary",
        r#"
            type Output { result: string }
        "#,
        example_service_impl(
            r#"
  function save(self: ExampleService, input: { value: string }) -> root.api.example.Output {
    return root.api.example.Output { result: "" }
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
        ),
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "anonymous record type",
            "cannot be used in service boundary schema",
        ],
    );
}

#[test]
fn request_local_exception_types_fail_contract_boundary_validation() {
    for (name, api_source, methods, expected) in [
        (
            "request-local-operation-param",
            r#"
                type Input {}
                type Output {}
                type LoginError implements ErrorPayload { message: string }
            "#,
            r#"
  function save(self: ExampleService, input: Exception<root.api.example.LoginError>) -> root.api.example.Output {
    return {}
  }
            "#,
            "request-local type Exception<...> cannot be used in service boundary schema",
        ),
        (
            "request-local-operation-return",
            r#"
                type Input {}
                type Output {}
                type LoginError implements ErrorPayload { message: string }
            "#,
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> CatchResult<root.api.example.Output, root.api.example.LoginError> {
    throw root.api.example.LoginError { message: "" }
  }
            "#,
            "request-local type CatchResult<...> cannot be used in service boundary schema",
        ),
        (
            "request-local-exported-field",
            r#"
                type Input { error: Exception<LoginError> }
                type Output {}
                type LoginError implements ErrorPayload { message: string }
            "#,
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
            "#,
            "request-local type Exception<...> cannot be used in service boundary schema",
        ),
        (
            "request-local-alias",
            r#"
                type Input = CatchResult<string, LoginError>
                type Output {}
                type LoginError implements ErrorPayload { message: string }
            "#,
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
            "#,
            "request-local type CatchResult<...> cannot be used in service boundary schema",
        ),
        (
            "request-local-nested-container",
            r#"
                type Input { nested: Array<Map<string, Exception<LoginError>>> }
                type Output {}
                type LoginError implements ErrorPayload { message: string }
            "#,
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
            "#,
            "request-local type Exception<...> cannot be used in service boundary schema",
        ),
        (
            "request-local-map-value",
            r#"
                type Input { values: Map<string, CatchResult<Output, LoginError>> }
                type Output {}
                type LoginError implements ErrorPayload { message: string }
            "#,
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
            "#,
            "request-local type CatchResult<...> cannot be used in service boundary schema",
        ),
        (
            "request-local-stream-chunk",
            r#"
                type Input {}
                type Output {}
                type LoginError implements ErrorPayload { message: string }
            "#,
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> Stream<Exception<root.api.example.LoginError>> {
    return
  }
            "#,
            "request-local type Exception<...> cannot be used in service boundary schema",
        ),
        (
            "request-local-union-member",
            r#"
                type Input = string | CatchResult<string, LoginError>
                type Output {}
                type LoginError implements ErrorPayload { message: string }
            "#,
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
            "#,
            "request-local type CatchResult<...> cannot be used in service boundary schema",
        ),
        (
            "request-local-anonymous-union-branch",
            r#"
                type Input discriminator "tag" =
                  { tag: "ok", value: string }
                  | { tag: "err", exception: Exception<LoginError> }
                type Output {}
                type LoginError implements ErrorPayload { message: string }
            "#,
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
            "#,
            "request-local type Exception<...> cannot be used in service boundary schema",
        ),
    ] {
        let temp = write_api_handler_project(name, api_source, example_service_impl(methods));
        let message = publish_error_message_without_package_dirs(temp.path());

        assert!(
            message.contains("contract validation failed"),
            "expected contract validation failure for {name}, got:\n{message}"
        );
        assert!(
            message.contains(expected),
            "expected error for {name} to contain {expected:?}, got:\n{message}"
        );
        assert!(
            !message.contains("standard_library type Exception is not schema-stable")
                && !message.contains("standard_library type CatchResult is not schema-stable"),
            "expected explicit request-local diagnostic for {name}, got:\n{message}"
        );
    }
}

#[test]
fn unknown_timeout_method_fails_publish() {
    let temp = write_example_api_handler_project_with_config(
        "unknown-timeout-method",
        r#"
id: example.com/example
version: 1.0.0
timeout:
  methods:
    ExampleService.missing: 1000
"#,
        default_api_source(),
        default_implementation_source(),
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "timeout.methods references unknown service operation ExampleService.missing",
        ],
    );
}

#[test]
fn stream_service_operation_publishes_as_server_stream() {
    let temp = write_api_handler_project(
        "stream-operation-runtime-unsupported",
        r#"
            type Input {}
            type Output {}
        "#,
        example_service_impl(
            r#"
  function save(self: ExampleService, input: root.api.example.Input) -> Stream<root.api.example.Output> {
    return
  }
"#,
        ),
    );

    let published = build_temp_service_publication(temp.path());
    let operation = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "ExampleService.save")
        .unwrap();
    let response_schema = serde_json::to_value(&operation.response).unwrap();
    let assembly_operation = service_operation(&published, "ExampleService.save");
    let service_unit_operation_abi = published.artifacts.service_unit.value["publicationAbi"]
        ["operationAbi"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"]["publicPath"] == "ExampleService.save")
        .unwrap();

    assert_eq!(operation.mode, "serverStream");
    assert_eq!(response_schema["xSkiffSymbol"], "api.example.Output");
    assert_eq!(assembly_operation["mode"], "serverStream");
    assert_eq!(
        assembly_operation["response"]["xSkiffSymbol"],
        "api.example.Output"
    );
    assert_eq!(
        assembly_operation["returnType"]["name"],
        "api.example.Output"
    );
    assert_eq!(
        service_unit_operation_abi["publicSignature"]["maySuspend"],
        true
    );
    assert_eq!(
        service_unit_operation_abi["publicSignature"]["returnType"]["name"],
        "Stream"
    );
    assert_eq!(
        service_unit_operation_abi["publicSignature"]["returnType"]["args"][0]["symbol"]
            ["modulePath"],
        "api.example"
    );
    assert_eq!(
        service_unit_operation_abi["publicSignature"]["returnType"]["args"][0]["symbol"]["symbol"],
        "Output"
    );
}

#[test]
fn pure_http_entry_without_exports_publishes_and_stays_out_of_service_exports() {
    let temp = TestDir::new("skiff-service-conformance", "http-entry-only");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http: internal.http.HttpEntry
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("http.skiff"),
        r#"
            type HttpEntry {}
            impl HttpEntry {
              function handle(self: HttpEntry, request: std.http.HttpRequest) -> std.http.HttpResponse {
                return std.http.noContent()
              }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());

    assert!(published.manifest.operations.iter().any(|operation| {
        operation.operation == "HttpEntry.handle"
            && operation.target == entry_target("example.com/example", "http.handle")
    }));
    assert_eq!(
        published
            .manifest
            .gateway
            .as_ref()
            .unwrap()
            .http
            .as_ref()
            .unwrap()
            .raw
            .as_ref()
            .unwrap()
            .operation,
        "HttpEntry.handle"
    );
    assert_eq!(
        published.artifacts.service_assembly.value["service"]["api"]["bindings"],
        serde_json::json!({})
    );
}

#[test]
fn websocket_entry_without_exports_projects_legacy_gateway_manifest() {
    let temp = TestDir::new("skiff-service-conformance", "websocket-entry-only");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.socket.SocketEntry
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("socket.skiff"),
        r#"
            type SocketEntry {}
            impl SocketEntry {
              function receive(self: SocketEntry, message: std.websocket.ConnectionMessage) -> null {
                return null
              }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();

    assert!(websocket.path.is_none());
    assert_eq!(websocket.service_param.as_deref(), Some("service"));
    assert_eq!(websocket.receive.operation, "SocketEntry.receive");
    assert_eq!(
        serde_json::to_value(&websocket.receive.adapter_args).unwrap(),
        serde_json::json!([
            { "param": "message", "source": { "kind": "websocket.message" } }
        ])
    );
    assert!(websocket.receive.gateway_entry_identity.is_some());
    assert_eq!(
        published.artifacts.service_assembly.value["service"]["api"]["bindings"],
        serde_json::json!({})
    );
}

#[test]
fn websocket_entry_context_schema_uses_implementation_source_types() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "websocket-entry-source-context",
    );
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.socket.SocketEntry
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("socket.skiff"),
        r#"
            type SocketEntry {}
            type ConnectionContext { userId: string }

            impl SocketEntry {
              function connect(self: SocketEntry, request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<ConnectionContext> {
                return { tag: "reject", code: 1000, reason: "" }
              }

              function receive(self: SocketEntry, context: ConnectionContext, message: std.websocket.ConnectionMessage) -> null {
                return null
              }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();
    let context_schema = serde_json::to_value(websocket.context.as_ref().unwrap()).unwrap();

    assert_eq!(
        context_schema["xSkiffSymbol"],
        "internal.socket.ConnectionContext"
    );
    assert_eq!(context_schema["properties"]["userId"]["type"], "string");

    let connect = service_operation(&published, "SocketEntry.connect");
    assert_eq!(
        connect["returnType"]["representation"]["items"][0]["fields"]["context"]["name"],
        "internal.socket.ConnectionContext"
    );
}

#[test]
fn websocket_entry_context_schema_uses_connect_return_type_ir() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "websocket-entry-generic-source-context",
    );
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.socket.SocketEntry
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("socket.skiff"),
        r#"
            type SocketEntry {}
            type ConnectionContext { userId: string }

            impl SocketEntry {
              function connect(self: SocketEntry, request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<Array<ConnectionContext>> {
                return { tag: "reject", code: 1000, reason: "" }
              }

              function receive(self: SocketEntry, context: Array<ConnectionContext>, message: std.websocket.ConnectionMessage) -> null {
                return null
              }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();
    let context_schema = serde_json::to_value(websocket.context.as_ref().unwrap()).unwrap();

    assert_eq!(context_schema["type"], "array");
    assert_eq!(
        context_schema["items"]["xSkiffSymbol"],
        "internal.socket.ConnectionContext"
    );
    assert_eq!(
        context_schema["items"]["properties"]["userId"]["type"],
        "string"
    );
}

#[test]
fn websocket_entry_context_schema_projects_private_discriminator_union() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "websocket-entry-source-union-context",
    );
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.socket.SocketEntry
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("socket.skiff"),
        r#"
            type SocketEntry {}
            type ConnectionContext discriminator "kind" =
              { kind: "user", userId: string }
              | { kind: "guest", guestId: string }

            impl SocketEntry {
              function connect(self: SocketEntry, request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<ConnectionContext> {
                return { tag: "reject", code: 1000, reason: "" }
              }

              function receive(self: SocketEntry, context: ConnectionContext, message: std.websocket.ConnectionMessage) -> null {
                return null
              }
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();
    let context_schema = serde_json::to_value(websocket.context.as_ref().unwrap()).unwrap();
    let branches = context_schema["oneOf"].as_array().unwrap();

    assert_eq!(
        context_schema["xSkiffSymbol"],
        "internal.socket.ConnectionContext"
    );
    assert_eq!(context_schema["xSkiffUnionDiscriminator"], "kind");
    assert!(branches.iter().any(|branch| {
        branch["properties"]["kind"]["enum"] == serde_json::json!(["user"])
            && branch["properties"]["userId"]["type"] == "string"
            && branch["xSkiffUnionBranch"] == "user"
    }));
    assert!(branches.iter().any(|branch| {
        branch["properties"]["kind"]["enum"] == serde_json::json!(["guest"])
            && branch["properties"]["guestId"]["type"] == "string"
            && branch["xSkiffUnionBranch"] == "guest"
    }));

    let connect = service_operation(&published, "SocketEntry.connect");
    let context_descriptor =
        &connect["returnType"]["representation"]["items"][0]["fields"]["context"];
    assert_eq!(context_descriptor["kind"], "representation");
    assert_eq!(
        context_descriptor["name"],
        "internal.socket.ConnectionContext"
    );
    assert_eq!(context_descriptor["discriminator"], "kind");
    assert_eq!(context_descriptor["representation"]["kind"], "union");
}

#[test]
fn websocket_entry_missing_receive_method_fails_publish() {
    let temp = write_example_api_handler_project_with_config(
        "websocket-missing-receive",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        default_api_source(),
        r#"
            type ExampleImpl {}
            impl ExampleImpl {
            }

            type ExampleService {}
            impl ExampleService {
            function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
              return {}
            }
            function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
              return Array.empty<root.api.example.Output>()
            }
            }
        "#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "websocket entry target internal.example.ExampleImpl",
            "missing method receive",
        ],
    );
}

#[test]
fn websocket_receive_business_parameter_projects_message_body_adapter_arg() {
    let temp = write_api_handler_project_with_config(
        "websocket-message-body-receive-parameter",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        r#"
            type Input {}
        "#,
        r#"
            type ExampleImpl {}
            impl ExampleImpl {
              function receive(self: ExampleImpl, input: root.api.example.Input) -> null {
                return null
              }
            }

            type ExampleService {}
            impl ExampleService {
              function receive(self: ExampleService, input: root.api.example.Input) -> null {
                return null
              }
            }
	"#,
    );

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&websocket.receive.adapter_args).unwrap(),
        serde_json::json!([
            { "param": "input", "source": { "kind": "websocket.messageBody" } }
        ])
    );
}

#[test]
fn websocket_connect_return_type_fails_publish() {
    let temp = write_api_handler_project_with_config(
        "websocket-connect-return-type",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        default_api_source(),
        r#"
            type ExampleImpl {}
            impl ExampleImpl {
              function connect(self: ExampleImpl, input: Input) -> Output {
                return {}
              }
              function receive(self: ExampleImpl, message: std.websocket.ConnectionMessage) -> null {
                return null
              }
            }
"#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "websocket entry connect method must return WebSocketConnectResult<T>",
        ],
    );
}

#[test]
fn websocket_receive_only_gateway_uses_service_param_default() {
    let temp = write_api_handler_project_with_config(
        "websocket-receive-only",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        default_api_source(),
        r#"
            type ExampleImpl {}
            impl ExampleImpl {
              function receive(self: ExampleImpl, message: std.websocket.ConnectionMessage) -> null {
                return null
              }
            }
"#,
    );

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();

    assert_eq!(websocket.service_param.as_deref(), Some("service"));
    assert!(websocket.context.is_none());
    assert!(websocket.connect.is_none());
    assert_eq!(websocket.receive.operation, "ExampleImpl.receive");
    let expected_receive_target = entry_target("example.com/example", "websocket.receive");
    assert_eq!(
        websocket.receive.service_operation_target.as_deref(),
        Some(expected_receive_target.as_str())
    );
    assert_eq!(
        websocket.receive.service_protocol_identity.as_deref(),
        Some(published.manifest.service.protocol_identity.as_str())
    );
    assert!(websocket.receive.gateway_entry_identity.is_some());
    assert!(websocket.gateway_entry_identity.is_some());
}

#[test]
fn websocket_routes_are_rejected_before_projection() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "websocket-routes-rejected-before-projection",
    );
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
websocket:
  connect: root.internal.socket.socketConnect
  routes:
    - path: /chat/send
      handler: root.internal.chat.send
  receive: root.internal.socket.receive
"#,
    )
    .unwrap();

    let error = read_service_config(temp.path()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("websocket routes are no longer supported"),
        "unexpected error: {error}"
    );
}

#[test]
fn websocket_receive_rejects_non_event_signature() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "websocket-receive-non-event-signature",
    );
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
websocket:
  connect: root.internal.socket.socketConnect
  receive: root.internal.socket.receive
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("socket.skiff"),
        r#"
            import std

            type ConnectionContext { userId: string }

            function socketConnect(request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<ConnectionContext> {
              return { tag: "reject", code: 1000, reason: "" }
            }

            function receive(identity: string, message: std.websocket.ConnectionMessage) -> null {
              return null
            }
        "#,
    )
    .unwrap();

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "must be function(event: std.websocket.WebSocketReceiveEvent<C>)",
        ],
    );
}

#[test]
fn websocket_receive_event_accepts_complex_connect_context_ir() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "websocket-receive-event-complex-context",
    );
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
websocket:
  connect: root.internal.socket.socketConnect
  receive: root.internal.socket.receive
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("socket.skiff"),
        r#"
            import std

            type ConnectionContext { userId: string }

            function socketConnect(request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<Array<ConnectionContext>> {
              return { tag: "reject", code: 1000, reason: "" }
            }

            function receive(event: std.websocket.WebSocketReceiveEvent<Array<ConnectionContext>>) -> null {
              return null
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();
    let context_schema = serde_json::to_value(websocket.context.as_ref().unwrap()).unwrap();

    assert_eq!(context_schema["type"], "array");
    assert_eq!(
        context_schema["items"]["xSkiffSymbol"],
        "internal.socket.ConnectionContext"
    );
}

#[test]
fn websocket_package_handlers_project_direct_package_targets_from_package_abi() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "websocket-package-handler-ir-projection",
    );
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: example.com/socket-kit
    version: 1.0.0
    alias: socketKit
websocket:
  connect: socketKit.Handler.connect
  receive: socketKit.receive
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "example.com/socket-kit",
        r#"
id: example.com/socket-kit
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "example.com/socket-kit",
        r#"
ConnectionContext: socket.ConnectionContext
Handler: socket.Handler
receive: socket.receive
        "#,
    );
    write_package_source(
        temp.path(),
        "example.com/socket-kit",
        "socket.skiff",
        r#"
          import std

          type ConnectionContext {
            userId: string,
            scopes: Array<string>
          }

          type Handler {}
          impl Handler {
            static function connect(request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<ConnectionContext> {
              return { tag: "reject", code: 1000, reason: "" }
            }
          }

          function receive(event: std.websocket.WebSocketReceiveEvent<ConnectionContext>) -> null {
            return null
          }
        "#,
    );

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();
    let context_schema = serde_json::to_value(websocket.context.as_ref().unwrap()).unwrap();
    let connect_target = "package.example%2Ecom%2Fsocket-kit.Handler%2Econnect";
    let receive_target = "package.example%2Ecom%2Fsocket-kit.receive";
    let connect_operation_manifest = published
        .manifest
        .operations
        .iter()
        .find(|operation| {
            operation.operation == "websocket.connect.package.socketKit.Handler.connect"
        })
        .expect("websocket package connect operation should be projected");
    let receive_operation_manifest = published
        .manifest
        .operations
        .iter()
        .find(|operation| operation.operation == "websocket.receive.package.socketKit.receive")
        .expect("websocket package receive operation should be projected");
    let package_type_source_paths = published
        .artifacts
        .file_ir_units
        .iter()
        .filter(|artifact| artifact.source_path.starts_with("__skiff/package_types/"))
        .map(|artifact| artifact.source_path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        websocket.connect.as_ref().unwrap().operation,
        "websocket.connect.package.socketKit.Handler.connect"
    );
    assert_eq!(
        websocket
            .connect
            .as_ref()
            .unwrap()
            .operation_abi_id
            .as_str(),
        connect_operation_manifest.operation_abi_id
    );
    assert!(
        !websocket
            .connect
            .as_ref()
            .unwrap()
            .operation_abi_id
            .is_empty(),
        "websocket package connect operation ABI id should be projected"
    );
    assert_eq!(
        websocket
            .connect
            .as_ref()
            .unwrap()
            .service_operation_target
            .as_deref(),
        Some(connect_target)
    );
    assert_eq!(
        websocket.receive.operation,
        "websocket.receive.package.socketKit.receive"
    );
    assert_eq!(
        websocket.receive.operation_abi_id,
        receive_operation_manifest.operation_abi_id
    );
    assert!(
        !websocket.receive.operation_abi_id.is_empty(),
        "websocket package receive operation ABI id should be projected"
    );
    assert_eq!(
        websocket.receive.service_operation_target.as_deref(),
        Some(receive_target)
    );
    assert_eq!(
        context_schema["xSkiffSymbol"], "__skiff.package_types.socketKit.ConnectionContext",
        "context_schema={context_schema:#}"
    );
    assert!(
        package_type_source_paths.is_empty(),
        "websocket package handlers should not publish package type mirror sources: {package_type_source_paths:?}"
    );
    assert!(
        published
            .artifacts
            .file_ir_units
            .iter()
            .all(|artifact| artifact.source_path != "__skiff/websocket_handlers.skiff"),
        "websocket package handlers should not publish generated service wrappers"
    );
    assert!(
        published
            .manifest
            .operations
            .iter()
            .any(|operation| operation.operation
                == "websocket.connect.package.socketKit.Handler.connect"
                && operation.target == connect_target),
        "websocket package connect operation should target package export"
    );
    assert!(
        published
            .manifest
            .operations
            .iter()
            .any(
                |operation| operation.operation == "websocket.receive.package.socketKit.receive"
                    && operation.target == receive_target
            ),
        "websocket package receive operation should target package export"
    );
}

#[test]
fn websocket_receive_rejects_nullable_connection_message_union_return() {
    let temp = write_api_handler_project_with_config(
        "websocket-receive-null-union-return",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        default_api_source(),
        r#"
            type ExampleImpl {}
            impl ExampleImpl {
              function receive(self: ExampleImpl, message: std.websocket.ConnectionMessage) -> std.websocket.ConnectionMessage | null {
                return null
              }
            }
	"#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "websocket entry receive method must return null or void",
        ],
    );
}

#[test]
fn websocket_receive_event_rejects_non_null_return() {
    let temp = TestDir::new(
        "skiff-service-conformance",
        "websocket-receive-event-non-null-return",
    );
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
websocket:
  connect: root.internal.socket.socketConnect
  receive: root.internal.socket.receive
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("socket.skiff"),
        r#"
            import std

            type ConnectionContext { userId: string }

            function socketConnect(request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<ConnectionContext> {
              return { tag: "reject", code: 1000, reason: "" }
            }

            function receive(event: std.websocket.WebSocketReceiveEvent<ConnectionContext>) -> string {
              return "not allowed"
            }
        "#,
    )
    .unwrap();

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "websocket entry receive method must return null or void",
        ],
    );
}

#[test]
fn websocket_receive_string_parameter_projects_message_body_adapter_arg() {
    let temp = write_api_handler_project_with_config(
        "websocket-string-message-body-bind",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        default_api_source(),
        r#"
            type ExampleImpl {}
            impl ExampleImpl {
              function receive(self: ExampleImpl, text: string) -> null {
                return null
              }
            }

            type ExampleService {}
            impl ExampleService {
              function receive(self: ExampleService, text: string) -> null {
                return null
              }
            }
	"#,
    );

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();

    assert_eq!(
        serde_json::to_value(&websocket.receive.adapter_args).unwrap(),
        serde_json::json!([
            { "param": "text", "source": { "kind": "websocket.messageBody" } }
        ])
    );
}

#[test]
fn websocket_receive_requires_message_body_message_or_receive_event() {
    let temp = write_api_handler_project_with_config(
        "websocket-receive-context-only",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        default_api_source(),
        r#"
            type ConnectionContext {}
            type ExampleImpl {}
            impl ExampleImpl {
              function connect(self: ExampleImpl, request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<ConnectionContext> {
                return { tag: "reject", code: 1000, reason: "" }
              }
              function receive(self: ExampleImpl, context: ConnectionContext) -> null {
                return null
              }
            }

            type ExampleService {}
            impl ExampleService {
              function receive(self: ExampleService, context: ConnectionContext) -> null {
                return null
              }
            }
	"#,
    );

    assert_publish_error_contains(
        temp.path(),
        &[
            "contract validation failed",
            "websocket entry receive must include a message, message body, or receive event parameter",
        ],
    );
}

#[test]
fn websocket_receive_number_parameter_projects_message_body_adapter_arg() {
    let temp = write_api_handler_project_with_config(
        "websocket-number-message-body-bind",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        default_api_source(),
        r#"
            type ExampleImpl {}
            impl ExampleImpl {
              function receive(self: ExampleImpl, text: number) -> null {
                return null
              }
            }

            type ExampleService {}
            impl ExampleService {
              function receive(self: ExampleService, text: number) -> null {
                return null
              }
            }
	"#,
    );

    let published = build_temp_service_publication(temp.path());
    let websocket = published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&websocket.receive.adapter_args).unwrap(),
        serde_json::json!([
            { "param": "text", "source": { "kind": "websocket.messageBody" } }
        ])
    );
}

#[test]
fn websocket_gateway_identity_ignores_unrelated_protocol_identity_changes() {
    let implementation_source = r#"
        type ExampleImpl {}
        impl ExampleImpl {
          function receive(self: ExampleImpl, message: std.websocket.ConnectionMessage) -> null {
            return null
          }
        }

        type ExampleService {}
        impl ExampleService {
          function extra(self: ExampleService) -> root.api.example.Extra {
            return { value: "" }
          }
          function receive(self: ExampleService, message: std.websocket.ConnectionMessage) -> null {
            return null
          }
        }
    "#;
    let first = write_api_handler_project_with_config(
        "websocket-identity-first",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        r#"
            type Extra { value: string }
        "#,
        implementation_source,
    );
    let second = write_api_handler_project_with_config(
        "websocket-identity-second",
        r#"
id: example.com/example
version: 1.0.0
websocket: internal.example.ExampleImpl
"#,
        r#"
            type Extra { value: string, changed: string? }
        "#,
        implementation_source,
    );

    let first_published = build_temp_service_publication(first.path());
    let second_published = build_temp_service_publication(second.path());
    let first_websocket = first_published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();
    let second_websocket = second_published
        .manifest
        .gateway
        .as_ref()
        .unwrap()
        .websocket
        .as_ref()
        .unwrap();

    assert_ne!(
        first_published.manifest.service.protocol_identity,
        second_published.manifest.service.protocol_identity
    );
    assert_eq!(
        first_websocket.gateway_entry_identity,
        second_websocket.gateway_entry_identity
    );
    assert_eq!(
        first_websocket.receive.gateway_entry_identity,
        second_websocket.receive.gateway_entry_identity
    );
}

fn write_example_api_handler_project(
    name: &str,
    implementation_source: impl AsRef<str>,
) -> TestDir {
    write_api_handler_project(name, default_api_source(), implementation_source)
}

fn write_legacy_mongo_service_project() -> TestDir {
    let temp = write_api_service_project(
        "legacy-mongo-provider",
        r#"
            type Output {}
            interface ExampleService {
              function ping() -> Output
            }
        "#,
        r#"
            import mongo

            type ExampleService {}

            impl ExampleService {
              function ping(self: ExampleService) -> root.api.example.Output {
                const target = mongo.Target("test-cluster", "example")
                return {}
              }
            }
        "#,
    );
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: skiff.run/mongo
    version: 1.0.0
    alias: mongo
"#,
    )
    .unwrap();
    write_package_manifest(
        temp.path(),
        "skiff.run/mongo",
        r#"
id: skiff.run/mongo
version: 1.0.0
"#,
    );
    write_package_api_yml(
        temp.path(),
        "skiff.run/mongo",
        r#"
MongoTarget: mongo.MongoTarget
"#,
    );
    write_package_source(
        temp.path(),
        "skiff.run/mongo",
        "mongo.skiff",
        r#"
provider mongo

export type MongoTarget {}
"#,
    );
    temp
}

fn write_api_handler_project(
    name: &str,
    api_source: impl AsRef<str>,
    implementation_source: impl AsRef<str>,
) -> TestDir {
    write_example_api_handler_project_with_config(
        name,
        default_service_config_with_module("internal.example"),
        api_source,
        implementation_source,
    )
}

fn write_api_handler_project_with_config(
    name: &str,
    service_config_source: impl AsRef<str>,
    api_source: impl AsRef<str>,
    implementation_source: impl AsRef<str>,
) -> TestDir {
    let temp = TestDir::new("skiff-service-conformance", name);
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        service_config_source.as_ref(),
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("example.skiff"),
        api_source.as_ref(),
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("example.skiff"),
        implementation_source.as_ref(),
    )
    .unwrap();
    write_example_api_yml(
        temp.path(),
        api_source.as_ref(),
        implementation_source.as_ref(),
    );
    temp
}

fn write_example_api_handler_project_with_config(
    name: &str,
    service_config_source: impl AsRef<str>,
    api_source: impl AsRef<str>,
    implementation_source: impl AsRef<str>,
) -> TestDir {
    write_api_handler_project_with_config(
        name,
        service_config_source,
        api_source,
        implementation_source,
    )
}

fn write_api_service_project(
    name: &str,
    api_source: impl AsRef<str>,
    implementation_source: impl AsRef<str>,
) -> TestDir {
    let temp = TestDir::new("skiff-service-conformance", name);
    fs::create_dir_all(temp.path().join("api")).unwrap();
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("api").join("example.skiff"),
        api_source.as_ref(),
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("example.skiff"),
        implementation_source.as_ref(),
    )
    .unwrap();
    write_example_api_yml(
        temp.path(),
        api_source.as_ref(),
        implementation_source.as_ref(),
    );
    temp
}

fn default_service_config_with_module(_module: &str) -> String {
    format!(
        r#"
id: example.com/example
version: 1.0.0
"#
    )
}

fn write_example_api_yml(root: &Path, api_source: &str, implementation_source: &str) {
    let service_names = surface_service_names(implementation_source).collect::<BTreeSet<_>>();
    if service_names.is_empty() {
        return;
    }

    let mut api_yml = String::new();
    for name in service_names {
        api_yml.push_str(&format!("{name}: internal.example.{name}\n"));
    }
    let names = top_level_decl_names(api_source).collect::<BTreeSet<_>>();
    if !names.is_empty() {
        api_yml.push_str("api:\n  example:\n");
        for name in names {
            api_yml.push_str(&format!("    {name}: api.example.{name}\n"));
        }
    }
    fs::write(root.join("api.yml"), api_yml).unwrap();
}

fn surface_service_names(source: &str) -> impl Iterator<Item = &str> {
    top_level_decl_names(source).filter(|name| name.ends_with("Service"))
}

fn top_level_decl_names(source: &str) -> impl Iterator<Item = &str> {
    let mut brace_depth = 0i32;
    source.lines().filter_map(move |line| {
        let line = line.trim_start();
        let rest = (brace_depth == 0)
            .then(|| {
                line.strip_prefix("type ")
                    .or_else(|| line.strip_prefix("alias "))
                    .or_else(|| line.strip_prefix("interface "))
                    .or_else(|| line.strip_prefix("function "))
                    .or_else(|| line.strip_prefix("const "))
            })
            .flatten();
        brace_depth += line.chars().filter(|ch| *ch == '{').count() as i32;
        brace_depth -= line.chars().filter(|ch| *ch == '}').count() as i32;
        rest.and_then(|rest| {
            rest.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .find(|name| !name.is_empty())
        })
    })
}

fn default_api_source() -> &'static str {
    r#"
        type Input {}
        type Output {}
    "#
}

fn example_service_impl(methods: &str) -> String {
    named_service_impl("ExampleService", methods)
}

fn named_service_impl(type_name: &str, methods: &str) -> String {
    format!(
        r#"
type {type_name} {{}}

impl {type_name} {{
{methods}
}}
"#
    )
}

fn default_implementation_source() -> String {
    example_service_impl(
        r#"
  function save(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return {}
  }
  function list(self: ExampleService, limit: number) -> Array<root.api.example.Output> {
    return Array.empty<root.api.example.Output>()
  }
"#,
    )
}

fn file_artifact_value(
    published: &skiff_compiler::BuiltServicePublication,
    source_path: &str,
) -> serde_json::Value {
    published
        .artifacts
        .file_ir_units
        .iter()
        .find(|artifact| artifact.source_path == source_path)
        .unwrap_or_else(|| panic!("file artifact {source_path} should be present"))
        .value()
}

fn declaration_function<'a>(artifact: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    executable_by_declaration(artifact, name)
}

fn code_function<'a>(artifact: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    executable_by_declaration(artifact, name)
}

fn executable_by_declaration<'a>(
    artifact: &'a serde_json::Value,
    name: &str,
) -> &'a serde_json::Value {
    let declaration = artifact["declarations"]["executables"][name]
        .as_object()
        .unwrap_or_else(|| panic!("executable declaration {name} should be present"));
    let index = declaration["executableIndex"]
        .as_u64()
        .unwrap_or_else(|| panic!("executable declaration {name} should have an index"))
        as usize;
    artifact["executables"]
        .as_array()
        .expect("executables should be an array")
        .get(index)
        .unwrap_or_else(|| panic!("executable {name} should be present at index {index}"))
}

fn assert_type_ref_resolves_to_builtin(
    artifact: &serde_json::Value,
    type_ref: &serde_json::Value,
    expected: &str,
) {
    match type_ref["kind"].as_str() {
        Some("builtin") => assert_eq!(type_ref["name"], expected),
        Some("localType") => {
            let declaration = local_type_declaration(artifact, type_ref);
            assert_eq!(declaration["descriptor"]["kind"], "alias");
            assert_type_ref_resolves_to_builtin(
                artifact,
                &declaration["descriptor"]["target"],
                expected,
            );
        }
        other => panic!("expected builtin-compatible type ref, got {other:?}: {type_ref}"),
    }
}

fn assert_type_ref_is_local_record_with_field(
    artifact: &serde_json::Value,
    type_ref: &serde_json::Value,
    expected_type: &str,
    expected_field: &str,
    expected_field_type: &str,
) {
    assert_eq!(type_ref["kind"], "localType");
    let declaration = local_type_declaration(artifact, type_ref);
    assert_eq!(declaration["name"], expected_type);
    assert_eq!(declaration["descriptor"]["kind"], "record");
    assert_type_ref_resolves_to_builtin(
        artifact,
        &declaration["descriptor"]["fields"][expected_field],
        expected_field_type,
    );
}

fn local_type_declaration<'a>(
    artifact: &'a serde_json::Value,
    type_ref: &serde_json::Value,
) -> &'a serde_json::Value {
    let index = type_ref["typeIndex"]
        .as_u64()
        .unwrap_or_else(|| panic!("local type ref should have typeIndex: {type_ref}"))
        as usize;
    artifact["typeTable"]
        .as_array()
        .expect("typeTable should be an array")
        .get(index)
        .unwrap_or_else(|| panic!("local type declaration should exist at index {index}"))
}

fn service_operation<'a>(
    published: &'a skiff_compiler::BuiltServicePublication,
    operation_name: &str,
) -> &'a serde_json::Value {
    published.artifacts.service_assembly.value["operations"]
        .as_array()
        .expect("service assembly operations should be an array")
        .iter()
        .find(|operation| operation["operation"] == operation_name)
        .unwrap_or_else(|| panic!("operation {operation_name} should be present"))
}

fn assert_closure_only_contract_type(schema: &serde_json::Value, source_symbol: &str) {
    assert!(
        schema["types"].get(source_symbol).is_some(),
        "closure-only type {source_symbol} should be present in contract schema"
    );
    assert!(
        schema["publicSymbolsBySource"].get(source_symbol).is_none(),
        "closure-only type {source_symbol} should not be externally writable"
    );
}

fn entry_target(service_id: &str, suffix: &str) -> String {
    let component = PublicationId::parse(service_id)
        .expect("test service id should parse")
        .runtime_target_component();
    format!("entry.{component}.{suffix}")
}
