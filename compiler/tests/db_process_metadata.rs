mod common;
use common::{artifacts::module_artifact, package_project::compile_package_project, TestDir};

#[test]
fn package_file_ir_contains_canonical_logical_db_schema() {
    let temp = package_with_source(
        "logical-db-schema",
        r#"
type Prompt {
  id: string,
  promptId: number,
  externalId: string?,
  secret: string
}

db object Prompt {
  primary key(id)
  storage secret using encrypted
  retention 180 days
  index byFeed(promptId desc, id desc)
  unique index byExternalId(externalId)
}
"#,
    );
    let project = compile_package_project(temp.path()).expect("package should compile");
    let declaration = module_artifact(&project.package, "main")
        .unit
        .declarations
        .db
        .get("Prompt")
        .expect("logical DB declaration should reach typed File IR");

    assert_eq!(declaration.type_name, "Prompt");
    assert_eq!(declaration.key.name, "id");
    assert_eq!(declaration.retention.as_ref().unwrap().amount, 180);
    assert_eq!(declaration.indexes.len(), 2);
    assert_eq!(declaration.indexes[0].name, "byFeed");
    assert!(declaration.indexes[1].unique);
    assert_eq!(
        declaration
            .fields
            .iter()
            .find(|field| field.name == "secret")
            .unwrap()
            .storage,
        skiff_artifact_model::DbFieldStorageIr::Encrypted
    );
}

#[test]
fn named_record_storage_types_are_expanded_in_logical_db_schema() {
    let temp = package_with_source(
        "logical-db-nested-types",
        r#"
type Window { resetAt: Date? }
type Quota { recoverAt: Date?, windows: Array<Window> }
type Source { id: string, createdAt: Date, quota: Quota? }
db object Source { primary key(id) }
"#,
    );
    let project = compile_package_project(temp.path()).expect("package should compile");
    let value = module_artifact(&project.package, "main").value();
    let fields = value["declarations"]["db"]["Source"]["fields"]
        .as_array()
        .expect("logical DB fields should serialize as an array");
    let quota = fields
        .iter()
        .find(|field| field["name"] == "quota")
        .expect("quota field");

    assert_eq!(quota["type"]["kind"], "nullable");
    assert_eq!(quota["type"]["inner"]["kind"], "record");
    assert_eq!(
        quota["type"]["inner"]["fields"]["recoverAt"]["inner"]["name"],
        "Date"
    );
    assert_eq!(
        quota["type"]["inner"]["fields"]["windows"]["args"][0]["fields"]["resetAt"]["inner"]
            ["name"],
        "Date"
    );
}

#[test]
fn invalid_logical_db_schema_is_rejected_during_package_compile() {
    for (name, source, expected) in [
        (
            "db-missing-key",
            "type Record { id: string } db object Record {}",
            "must declare key",
        ),
        (
            "db-unknown-key",
            "type Record { id: string } db object Record { primary key(missing) }",
            "primary key field missing",
        ),
        (
            "db-reserved-key",
            "type Record { _id: string } db object Record { primary key(_id) }",
            "reserved _id field",
        ),
        (
            "db-duplicate-index",
            r#"
type Record { id: string, owner: string }
db object Record {
  primary key(id)
  index byOwner(owner)
  index byOwner(id)
}
"#,
            "index name byOwner is declared more than once",
        ),
        (
            "db-invalid-index-path",
            r#"
type Record { id: string, owner: string }
db object Record { primary key(id); index byOwner(owner.missing) }
"#,
            "cannot traverse non-record field owner",
        ),
    ] {
        let error = compile_package_project(package_with_source(name, source).path())
            .expect_err("invalid logical DB schema should fail")
            .to_string();
        assert!(
            error.contains(expected),
            "expected {expected:?} in compile error: {error}"
        );
    }
}

#[test]
fn package_db_read_write_operations_compile_through_canonical_file_ir() {
    let temp = package_with_source(
        "logical-db-operations",
        r#"
type Credential { id: string, apiKey: string, label: string? }
db object Credential {
  primary key(id)
  storage apiKey using encrypted
}

function write(id: string, value: string) -> bool {
  db insert Credential { id = id apiKey = value }
  db require Credential(id) { fields { apiKey } }
  db update Credential(id) { apiKey = value }
  db replace Credential(id) { apiKey = value }
  db upsert Credential(id) { apiKey = value } { apiKey = value }
  return true
}

function scan(lastId: string) -> Array<Credential> {
  return db find many Credential {
    where id > lastId
    order id asc
    limit 100
  }
}
"#,
    );
    let project = compile_package_project(temp.path()).expect("DB operations should compile");
    let value = module_artifact(&project.package, "main").value();
    let executable_text = serde_json::to_string(&value["executables"]).unwrap();
    assert!(executable_text.contains("dbOperation"));
    assert!(executable_text.contains("Credential"));
}

#[test]
fn package_db_bodies_reject_missing_key_and_illegal_changes() {
    for (name, operation, expected) in [
        (
            "db-insert-missing-field",
            "db insert Record { id = id }",
            "missing required field `value`",
        ),
        (
            "db-insert-missing-key",
            "db insert Record { value = value }",
            "missing required field `id`",
        ),
        (
            "db-replace-key-in-body",
            "db replace Record(id) { id = id value = value }",
            "replace by key body cannot include key field `id`",
        ),
        (
            "db-upsert-key-in-body",
            "db upsert Record(id) { id = id value = value } { value = value }",
            "upsert by key insert body cannot include key field `id`",
        ),
        (
            "db-update-key",
            "db update Record(id) { id = value }",
            "cannot modify key field `id`",
        ),
        (
            "db-update-unknown-field",
            "db update Record(id) { missing = value }",
            "unknown field `missing`",
        ),
    ] {
        let source = format!(
            r#"
type Record {{ id: string, value: string }}
db object Record {{ primary key(id) }}
function write(id: string, value: string) -> bool {{
  {operation}
  return true
}}
"#
        );
        assert_compile_error_contains(name, &source, expected);
    }
}

#[test]
fn encrypted_storage_rejects_query_and_partial_change_use() {
    for (name, operation, expected) in [
        (
            "encrypted-predicate",
            "db find many Credential { where apiKey == value }",
            "encrypted storage field `apiKey` cannot be used for predicate",
        ),
        (
            "encrypted-order",
            "db find many Credential { order apiKey asc }",
            "encrypted storage field `apiKey` cannot be used for order",
        ),
        (
            "encrypted-query-update",
            "db update many Credential { where id != value } { apiKey = value }",
            "cannot be used for whole-field set without a key selector",
        ),
        (
            "encrypted-partial-change",
            "db update Credential(id) { apiKey += value }",
            "cannot be used for partial change",
        ),
    ] {
        let source = format!(
            r#"
type Credential {{ id: string, apiKey: string }}
db object Credential {{
  primary key(id)
  storage apiKey using encrypted
}}
function run(id: string, value: string) -> bool {{
  {operation}
  return true
}}
"#
        );
        assert_compile_error_contains(name, &source, expected);
    }
}

#[test]
fn ordinary_logical_db_keeps_non_string_primary_keys() {
    let temp = package_with_source(
        "logical-db-number-key",
        r#"
type Counter { id: number, value: string }
db object Counter { primary key(id) }
function write(value: string) -> bool {
  db insert Counter { id = 1 value = value }
  db require Counter(1)
  db update Counter(1) { value = value }
  return true
}
"#,
    );
    compile_package_project(temp.path()).expect("number-key DB operations should compile");
}

fn package_with_source(name: &str, source: &str) -> TestDir {
    let temp = TestDir::new("skiff-compiler", name);
    temp.write(
        "package.yml",
        "id: example.com/db-fixture\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "{}\n");
    temp.write("main.skiff", source);
    temp
}

fn assert_compile_error_contains(name: &str, source: &str, expected: &str) {
    let temp = package_with_source(name, source);
    let error = compile_package_project(temp.path())
        .expect_err("invalid DB operation should fail")
        .to_string();
    assert!(
        error.contains(expected),
        "expected {expected:?} in compile error: {error}"
    );
}
