use skiff_compiler::{
    read_service_config,
    test_support::compile_source_file_ir_artifact_for_test as compile_source_file_ir_artifact,
    test_support::project_fixtures::{write_package_api_yml, ServiceProjectBuilder},
};

mod common;
use common::artifacts::{
    assert_publish_error_contains, build_temp_service_publication, package_source_artifact,
    source_artifact,
};

#[test]
fn build_service_publication_assembly_includes_db_object_metadata() {
    let project = service_project(
        "db-object-metadata",
        r#"
            type Message {
              id: string,
              promptId: string,
              serverAt: number
            }

            db object Message {
              name "message"
              primary key(id)
            }

            type Prompt {
              id: string,
              promptIdNumber: number,
              externalId: string?
            }

            db object Prompt {
              name "prompt"
              primary key(id)
              retention 180 days
              index byFeed(promptIdNumber desc, id desc)
              unique index byExternalId(externalId) where externalId != null
            }
        "#,
    );

    let published = build_temp_service_publication(project.root());
    let assembly = &published.artifacts.service_assembly.value;
    let service_unit = &published.artifacts.service_unit.value;
    let db = assembly["db"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["typeName"] == "Prompt")
        .expect("Prompt db metadata");

    assert_eq!(db["typeName"], "Prompt");
    assert_eq!(db["collectionName"], "prompt");
    assert_eq!(db["kind"], "object");
    assert_eq!(
        db["retention"],
        serde_json::json!({ "amount": 180, "unit": "days" })
    );
    assert_eq!(db["key"]["name"], "id");
    assert!(db.get("relations").is_none());
    assert_eq!(db["indexes"][0]["name"], "byFeed");
    assert_eq!(db["indexes"][0]["fields"][0]["direction"], "desc");
    assert_eq!(db["indexes"][1]["name"], "byExternalId");
    assert!(db["indexes"][1]["unique"].as_bool().unwrap());
    assert_eq!(db["indexes"][1]["where"]["Binary"]["op"], "Ne");
    assert_eq!(
        db["indexes"][1]["where"]["Binary"]["left"]["Identifier"],
        "externalId"
    );
    assert_eq!(service_unit["db"], assembly["db"]);

    let artifact = source_artifact(&published, "internal/example.skiff");
    let artifact_value = artifact.value();
    let file_db = &artifact_value["declarations"]["db"]["Prompt"];
    assert_eq!(file_db["collectionName"], "prompt");
    assert_eq!(file_db["kind"], "object");
    assert_eq!(
        file_db["retention"],
        serde_json::json!({ "amount": 180, "unit": "days" })
    );
    assert_eq!(file_db["key"]["name"], "id");
    assert!(file_db.get("relations").is_none());
    assert_eq!(file_db["indexes"][1]["where"]["Binary"]["op"], "Ne");
}

#[test]
fn db_metadata_expands_named_storage_types_for_nested_date_fields() {
    let project = service_project(
        "db-storage-date-metadata",
        r#"
            type QuotaWindow {
              resetAt: Date?
            }

            type QuotaState {
              recoverAt: Date?,
              windows: Array<QuotaWindow>
            }

            type Source {
              id: string,
              createdAt: Date,
              quota: QuotaState?
            }

            db object Source {
              name "source"
              primary key(id)
            }
        "#,
    );

    let published = build_temp_service_publication(project.root());
    let service_unit = &published.artifacts.service_unit.value;
    let db = service_unit["db"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["typeName"] == "Source")
        .expect("Source db metadata");
    let fields = db["fields"]
        .as_array()
        .expect("db fields should be an array");
    let created_at = fields
        .iter()
        .find(|field| field["name"] == "createdAt")
        .expect("createdAt field metadata");
    assert_eq!(
        created_at["type"],
        serde_json::json!({ "kind": "builtin", "name": "Date" })
    );

    let quota = fields
        .iter()
        .find(|field| field["name"] == "quota")
        .expect("quota field metadata");
    let quota_inner = &quota["type"]["inner"];
    assert_eq!(quota["type"]["kind"], "nullable");
    assert_eq!(quota_inner["kind"], "record");
    assert_eq!(
        quota_inner["fields"]["recoverAt"],
        serde_json::json!({
            "kind": "nullable",
            "inner": { "kind": "builtin", "name": "Date" }
        })
    );
    assert_eq!(
        quota_inner["fields"]["windows"]["args"][0]["fields"]["resetAt"],
        serde_json::json!({
            "kind": "nullable",
            "inner": { "kind": "builtin", "name": "Date" }
        })
    );
}

#[test]
fn publish_rejects_invalid_db_metadata() {
    for (name, source, expected) in [
        (
            "db-key-missing-attached-type-field",
            r#"
                type Thread { id: string }
                db object Thread { name "thread"; primary key(missing) }
            "#,
            vec!["db object Thread primary key field missing must be a field on the attached type"],
        ),
        (
            "db-missing-key",
            r#"
                type Thread { id: string, ownerUserId: string }
                db object Thread { name "thread" }
            "#,
            vec!["db object Thread must declare key"],
        ),
        (
            "db-id-key",
            r#"
                type Thread { _id: string }
                db object Thread { name "thread"; primary key(_id) }
            "#,
            vec!["db object Thread key cannot use reserved _id field"],
        ),
        (
            "db-duplicate-index",
            r#"
                type Thread {
                  id: string,
                  ownerUserId: string
                }

                db object Thread {
                  name "thread"
                  primary key(id)
                  index byOwner(ownerUserId)
                  index byOwner(id)
                }
            "#,
            vec!["db object Thread index name byOwner is declared more than once"],
        ),
        (
            "db-empty-index",
            r#"
                type Thread { id: string }

                db object Thread {
                  name "thread"
                  primary key(id)
                  index empty()
                }
            "#,
            vec!["db object Thread index empty must declare at least one field"],
        ),
        (
            "db-empty-object-name",
            r#"
                type Thread { id: string }

                db object Thread {
                  name ""
                  primary key(id)
                }
            "#,
            vec!["db object Thread name cannot be empty"],
        ),
        (
            "db-plural-object-name",
            r#"
                type Thread { id: string }

                db object Thread {
                  name "threads"
                  primary key(id)
                }
            "#,
            vec!["db object Thread name threads must be singular"],
        ),
        (
            "db-index-where-unknown-field",
            r#"
                type Thread {
                  id: string,
                  externalId: string?
                }

                db object Thread {
                  name "thread"
                  primary key(id)
                  unique index byExternalId(externalId) where missing != null
                }
            "#,
            vec!["db object Thread index byExternalId where missing on Thread references unknown field missing"],
        ),
        (
            "db-index-where-unknown-field-named-messages",
            r#"
                type Thread { id: string }

                db object Thread {
                  name "thread"
                  primary key(id)
                  index byRelation(id) where messages != null
                }
            "#,
            vec!["db object Thread index byRelation where messages on Thread references unknown field messages"],
        ),
        (
            "db-index-where-unknown-nested-field",
            r#"
                type Owner { id: string }
                type Thread {
                  id: string,
                  owner: Owner
                }

                db object Thread {
                  name "thread"
                  primary key(id)
                  index byOwner(id) where owner.missing != null
                }
            "#,
            vec!["db object Thread index byOwner where owner.missing on Thread references unknown field missing"],
        ),
    ] {
        let project = service_project(name, source);
        assert_publish_error_contains(project.root(), &expected);
    }
}

#[test]
fn publish_rejects_db_object_attached_type_with_interface_field() {
    let project = service_project(
        "db-object-interface-field",
        r#"
            interface Store {
              function save(self: Self, input: string) -> void
            }

            type Thread {
              id: string,
              store: Store
            }

            db object Thread {
              name "thread"
              primary key(id)
            }
        "#,
    );

    assert_publish_error_contains(
        project.root(),
        &["type `Thread` field `store` uses interface type `Store`"],
    );
}

#[test]
fn publish_validates_nested_db_field_paths() {
    let valid = service_project(
        "db-valid-nested-field",
        r#"
            type User { id: string }
            type Thread {
              id: string,
              owner: User
            }

            db object Thread {
              name "thread"
              primary key(id)
              index byOwner(owner.id)
            }
        "#,
    );
    build_temp_service_publication(valid.root());

    let missing = service_project(
        "db-missing-nested-field",
        r#"
            type User { id: string }
            type Thread {
              id: string,
              owner: User
            }

            db object Thread {
              name "thread"
              primary key(id)
              index byOwner(owner.missing)
            }
        "#,
    );
    assert_publish_error_contains(
        missing.root(),
        &["db object index owner.missing on Thread references unknown field missing"],
    );

    let non_record = service_project(
        "db-non-record-nested-field",
        r#"
            type Thread { id: string }

            db object Thread {
              name "thread"
              primary key(id)
              index byId(id.value)
            }
        "#,
    );
    assert_publish_error_contains(
        non_record.root(),
        &["db object index id.value on Thread cannot traverse non-record field id"],
    );
}

#[test]
fn db_rejects_relation_declaration_in_object_db_v1() {
    let project = service_project(
        "db-relation-declaration-unsupported",
        r#"
            type Message {
              id: string,
              threadId: number
            }

            db object Message {
              name "message"
              primary key(id)
            }
            type Thread { id: string }

            db object Thread {
              name "thread"
              primary key(id)
              relation messages: Message many { match threadId }
            }
        "#,
    );

    assert_publish_error_contains(
        project.root(),
        &["db object relation declarations are not supported in object DB v1"],
    );
}

#[test]
fn publish_rejects_collection_name_conflicts_and_reserved_prefix() {
    let duplicate = service_project(
        "db-duplicate-collection",
        r#"
            type Thread { id: string }
            type ApiThread { id: string }
            db object Thread { name "thread"; primary key(id) }
            db object ApiThread { name "thread"; primary key(id) }
        "#,
    );
    assert_publish_error_contains(duplicate.root(), &["db object name thread is used by both"]);

    let reserved = service_project(
        "db-reserved-collection",
        r#"
            type Internal { id: string }
            db object Internal { name "_skiff_internal"; primary key(id) }
        "#,
    );
    assert_publish_error_contains(
        reserved.root(),
        &["db object Internal name _skiff_internal uses reserved _skiff_ prefix"],
    );
}

#[test]
fn package_collection_name_mapping_rewrites_service_db_metadata_only() {
    let project = package_collection_mapping_project(
        "package-collection-mapping",
        r#"
packages:
  - id: skiff.run/http-session
    version: 1.0.0
    alias: httpSession
    collection_name_mapping:
      Session: registry_session
"#,
        "",
    );

    let published = build_temp_service_publication(project.root());
    let assembly = &published.artifacts.service_assembly.value;
    let service_unit = &published.artifacts.service_unit.value;
    let package_db = assembly["db"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["packageId"] == "skiff.run/http-session")
        .expect("package db metadata");

    assert_eq!(package_db["typeName"], "Session");
    assert_eq!(package_db["collectionName"], "registry_session");
    assert_eq!(service_unit["db"], assembly["db"]);
    assert!(service_unit["packageDependencies"][0]
        .get("collectionNameMapping")
        .is_none());

    let file_ir = package_source_artifact(&published, "session_impl.skiff");
    assert_eq!(
        file_ir.value()["declarations"]["db"]["Session"]["collectionName"],
        "Session"
    );
    assert!(published.artifacts.package_units[0]
        .value
        .get("collectionNameMapping")
        .is_none());
}

#[test]
fn package_collection_name_mapping_is_validated_against_package_metadata() {
    for (name, packages_yaml, service_db, expected) in [
        (
            "unknown-key",
            r#"
packages:
  - id: skiff.run/http-session
    version: 1.0.0
    alias: httpSession
    collection_name_mapping:
      internal.session.Session: registry_session
"#,
            "",
            "collection_name_mapping key internal.session.Session does not match package db collectionName",
        ),
        (
            "reserved-value",
            r#"
packages:
  - id: skiff.run/http-session
    version: 1.0.0
    alias: httpSession
    collection_name_mapping:
      Session: _skiff_session
"#,
            "",
            "collection_name_mapping Session value _skiff_session uses reserved _skiff_ prefix",
        ),
        (
            "plural-value",
            r#"
packages:
  - id: skiff.run/http-session
    version: 1.0.0
    alias: httpSession
    collection_name_mapping:
      Session: sessions
"#,
            "",
            "collection_name_mapping Session value sessions must be singular",
        ),
        (
            "service-conflict",
            r#"
packages:
  - id: skiff.run/http-session
    version: 1.0.0
    alias: httpSession
    collection_name_mapping:
      Session: thread
"#,
            r#"type Thread { id: string } db object Thread { name "thread"; primary key(id) }"#,
            "db collectionName thread is used by both service internal.example.Thread and package skiff.run/http-session session_impl.Session",
        ),
    ] {
        let project = package_collection_mapping_project(name, packages_yaml, service_db);
        assert_publish_error_contains(project.root(), &[expected]);
    }
}

#[test]
fn publish_projects_url_like_service_ids_to_mongo_database_names_when_metadata_exists() {
    let project = service_project_with_id(
        "db-url-like-service-id",
        "google.com/cloud",
        r#"
            type Thread { id: string }
            db object Thread { name "thread"; primary key(id) }
        "#,
    );

    build_temp_service_publication(project.root());
}

#[test]
fn publish_rejects_reserved_simple_service_ids_before_mongo_projection() {
    let project = service_project_with_id(
        "db-reserved-service-id",
        "admin",
        r#"
            type Thread { id: string }
            db object Thread { name "thread"; primary key(id) }
        "#,
    );

    let error = read_service_config(project.root()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("publication id uses an unsafe path-like form"),
        "unexpected error: {error}"
    );
}

#[test]
fn object_db_insert_body_requires_key_and_required_stored_fields() {
    assert_compile_error_contains(
        r#"
            type User {
              id: string,
              displayName: string,
              nickname: string?
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(id: string) -> bool {
                db insert User { id = id }
                return true
            }
        "#,
        "db insert body missing required field `displayName` on User",
    );

    assert_compile_error_contains(
        r#"
            type User {
              id: string,
              displayName: string
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(displayName: string) -> bool {
                db insert User { displayName = displayName }
                return true
            }
        "#,
        "db insert body missing required field `id` on User",
    );
}

#[test]
fn object_db_replace_body_uses_selector_key_rules() {
    compile_source_file_ir_artifact(
        r#"
            type User {
              id: string,
              displayName: string,
              nickname: string?
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(id: string) -> bool {
                db replace User(id) { displayName = "Ada" }
                return true
            }
        "#,
        "internal/object_db_replace_ok.skiff",
        "internal.object_db_replace_ok",
        "service",
    )
    .expect("replace by key should not require key in body");

    assert_compile_error_contains(
        r#"
            type User {
              id: string,
              displayName: string
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(id: string) -> bool {
                db replace User(id) { id = id displayName = "Ada" }
                return true
            }
        "#,
        "db replace by key body cannot include key field `id` on User",
    );

    assert_compile_error_contains(
        r#"
            type User {
              id: string,
              displayName: string
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(oldName: string) -> bool {
                db replace User { where displayName == oldName } { displayName = "Ada" }
                return true
            }
        "#,
        "db replace by query body missing required field `id` on User",
    );
}

#[test]
fn object_db_upsert_insert_body_uses_selector_key_rules() {
    compile_source_file_ir_artifact(
        r#"
            type User {
              id: string,
              displayName: string,
              nickname: string?
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(id: string) -> bool {
                db upsert User(id) { displayName = "Ada" } { displayName = "Grace" }
                return true
            }
        "#,
        "internal/object_db_upsert_ok.skiff",
        "internal.object_db_upsert_ok",
        "service",
    )
    .expect("upsert by key should use selector key for insert body");

    assert_compile_error_contains(
        r#"
            type User {
              id: string,
              displayName: string
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(id: string) -> bool {
                db upsert User(id) { id = id displayName = "Ada" } { displayName = "Grace" }
                return true
            }
        "#,
        "db upsert by key insert body cannot include key field `id` on User",
    );
}

#[test]
fn object_db_change_rejects_key_unknown_and_nested_paths() {
    assert_compile_error_contains(
        r#"
            type Thread {
              id: string,
              title: string
            }

            db object Thread {
              name "thread"
              primary key(id)
            }

            function run(id: string) -> bool {
                db update Thread(id) { id = "next" }
                return true
            }
        "#,
        "db change block cannot modify key field `id` on Thread",
    );

    assert_compile_error_contains(
        r#"
            type Thread {
              id: string,
              title: string
            }

            db object Thread {
              name "thread"
              primary key(id)
            }

            function run(id: string) -> bool {
                db update Thread(id) { messages = "next" }
                return true
            }
        "#,
        "db change block references unknown field `messages` on Thread",
    );

    assert_compile_error_contains(
        r#"
            type Profile { name: string }
            type User {
              id: string,
              profile: Profile
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(id: string) -> bool {
                db update User(id) { unset profile; unset profile.name }
                return true
            }
        "#,
        "db change field path `profile.name` on User must be a top-level stored field",
    );
}

#[test]
fn object_db_change_rejects_incompatible_atomic_operator_field_types() {
    assert_compile_error_contains(
        r#"
            type User {
              id: string,
              displayName: string,
              visits: number
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(id: string) -> bool {
                db update User(id) { displayName += 1 }
                return true
            }
        "#,
        "db change operator +=/-= requires numeric field `displayName` on User",
    );

    assert_compile_error_contains(
        r#"
            type User {
              id: string,
              displayName: string,
              tags: Array<string>
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(id: string) -> bool {
                db update User(id) { add displayName "admin" }
                return true
            }
        "#,
        "db change add/remove requires array field `displayName` on User",
    );
}

fn service_project(name: &str, internal_source: &str) -> ServiceProjectBuilder {
    service_project_with_id(name, "example.com/example", internal_source)
}

fn service_project_with_id(
    name: &str,
    service_id: &str,
    internal_source: &str,
) -> ServiceProjectBuilder {
    ServiceProjectBuilder::new(name)
        .write_root_file(
            "service.yml",
            &format!(
                r#"
id: {service_id}
version: 1.0.0
"#
            ),
        )
        .write_root_file(
            "api.yml",
            r#"
ExampleService: internal.handler.ExampleService
api:
  example:
    Input: api.example.Input
    Output: api.example.Output
    Thread: api.example.Thread
    ExampleService: api.example.ExampleService
        "#,
        )
        .write_source(
            "api/example.skiff",
            r#"
            type Input {}
            type Output {}
            type Thread { id: string }
            interface ExampleService {
              function run(input: Input) -> Output
            }
        "#,
        )
        .write_source("internal/example.skiff", internal_source)
        .write_source(
            "internal/handler.skiff",
            r#"
            function run(input: root.api.example.Input) -> root.api.example.Output {
              return root.api.example.Output {}
            }

            type ExampleService {}

            impl ExampleService {
              function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
                return root.internal.handler.run(input)
              }
            }
        "#,
        )
}

fn package_collection_mapping_project(
    name: &str,
    packages_yaml: &str,
    service_db: &str,
) -> ServiceProjectBuilder {
    let project = ServiceProjectBuilder::new(name)
        .write_root_file(
            "service.yml",
            &format!(
                r#"
id: example.com/example
version: 1.0.0
{packages_yaml}
"#
            ),
        )
        .write_root_file(
            "api.yml",
            r#"
ExampleService: internal.handler.ExampleService
api:
  example:
    Input: api.example.Input
    Output: api.example.Output
    ExampleService: api.example.ExampleService
        "#,
        )
        .write_source(
            "api/example.skiff",
            r#"
            type Input {}
            type Output {}
            interface ExampleService {
              function run(input: Input) -> Output
            }
        "#,
        )
        .write_source(
            "internal/example.skiff",
            &format!(
                r#"
            {service_db}
        "#
            ),
        )
        .write_source(
            "internal/handler.skiff",
            r#"
            function run(input: root.api.example.Input) -> root.api.example.Output {
              return root.api.example.Output {}
            }

            type ExampleService {}

            impl ExampleService {
              function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
                return root.internal.handler.run(input)
              }
            }
        "#,
        );
    project.add_package_manifest_in_dir(
        "skiff.run/http-session",
        r#"
id: skiff.run/http-session
version: 1.0.0
"#,
    );
    write_package_api_yml(
        project.root(),
        "skiff.run/http-session",
        r#"
session:
  Session: session_impl.Session
"#,
    );
    project.add_package_source(
        "skiff.run/http-session",
        "session_impl.skiff",
        r#"
        type Session {
          id: string
        }

        db object Session {
          primary key(id)
        }
        "#,
    );
    project
}

fn compile_error(source: &str) -> String {
    compile_source_file_ir_artifact(
        source,
        "internal/object_db.skiff",
        "internal.object_db",
        "service",
    )
    .expect_err("fixture should fail semantic validation")
    .to_string()
}

fn assert_compile_error_contains(source: &str, expected: &str) {
    let error = compile_error(source);
    assert!(
        error.contains(expected),
        "expected error to contain {expected:?}, got:\n{error}"
    );
}
