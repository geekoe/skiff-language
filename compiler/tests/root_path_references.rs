mod common;

use std::fs;

use common::{artifacts::module_artifact, package_project::compile_package_project, TestDir};

#[test]
fn root_path_resolves_unexported_internal_type_over_local_same_name() {
    let temp = TestDir::new("skiff-compiler", "root-type-reference");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("package.yml"),
        "id: example.com/root-type-reference\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("internal/helpers.skiff"),
        "type Helper { value: string }\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("internal/consumer.skiff"),
        r#"
type Helper { local: string }
type Holder { helper: root.internal.helpers.Helper }

function hold(value: root.internal.helpers.Helper) -> Holder {
  return Holder { helper: value }
}
"#,
    )
    .unwrap();

    let project = compile_package_project(temp.path()).expect("root reference should compile");
    let helper = module_artifact(&project.package, "internal.helpers").value();
    let consumer = module_artifact(&project.package, "internal.consumer").value();
    let helper_index = declared_type_index(&helper, "Helper");

    assert_json_contains_publication_type(&consumer, "internal.helpers", helper_index);
    assert!(!consumer
        .to_string()
        .contains("root.internal.helpers.Helper"));
    assert!(project
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .is_empty());
}

#[test]
fn root_path_resolves_attached_db_object_type_in_file_ir() {
    let temp = TestDir::new("skiff-compiler", "root-db-reference");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("package.yml"),
        "id: example.com/root-db-reference\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("internal/models.skiff"),
        r#"
type Thread { id: string, ownerUserId: string }
db object Thread { name "thread" primary key(id) }
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal/consumer.skiff"),
        "type Holder { thread: root.internal.models.Thread }\n",
    )
    .unwrap();

    let project = compile_package_project(temp.path()).expect("DB root reference should compile");
    let model = module_artifact(&project.package, "internal.models").value();
    let consumer = module_artifact(&project.package, "internal.consumer").value();
    let thread_index = declared_type_index(&model, "Thread");

    assert_eq!(
        model["declarations"]["db"]["Thread"]["typeRef"],
        serde_json::json!({
            "kind": "dbObjectSymbol",
            "symbol": { "modulePath": "internal.models", "symbol": "Thread" }
        })
    );
    assert_json_contains_publication_type(&consumer, "internal.models", thread_index);
}

#[test]
fn unknown_root_module_and_symbol_fail_package_compilation() {
    let cases = [
        (
            "missing-module",
            "type Holder { value: root.internal.missing.Helper }\n",
            &[
                "invalid root reference",
                "root.internal.missing.Helper",
                "internal/missing.skiff",
            ][..],
        ),
        (
            "missing-symbol",
            "type Holder { value: root.internal.helpers.Missing }\n",
            &[
                "invalid root reference",
                "root.internal.helpers.Missing",
                "Missing",
            ][..],
        ),
    ];

    for (name, source, fragments) in cases {
        let temp = TestDir::new("skiff-compiler", name);
        fs::create_dir_all(temp.path().join("internal")).unwrap();
        fs::write(
            temp.path().join("package.yml"),
            format!("id: example.com/{name}\nversion: 1.0.0\n"),
        )
        .unwrap();
        fs::write(
            temp.path().join("internal/helpers.skiff"),
            "type Helper { value: string }\n",
        )
        .unwrap();
        fs::write(temp.path().join("consumer.skiff"), source).unwrap();

        let error = compile_package_project(temp.path())
            .expect_err("invalid root reference should fail")
            .to_string();
        for fragment in fragments {
            assert!(
                error.contains(fragment),
                "expected {name} error to contain {fragment:?}, got:\n{error}"
            );
        }
    }
}

#[test]
fn test_only_files_neither_enter_nor_satisfy_package_file_ir() {
    let ignored = TestDir::new("skiff-compiler", "ignored-test-root");
    fs::write(
        ignored.path().join("package.yml"),
        "id: example.com/ignored-test-root\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(ignored.path().join("main.skiff"), "type Main {}\n").unwrap();
    fs::write(
        ignored.path().join("broken.test.skiff"),
        r#"
test "test-only root reference" {
  const missing: root.internal.missing.Helper = root.internal.missing.Helper { value: "hi" }
  assert true
}
"#,
    )
    .unwrap();

    let project = compile_package_project(ignored.path()).expect("test file should be excluded");
    assert!(project
        .package
        .file_ir_units
        .iter()
        .all(|file| !file.source_path.ends_with(".test.skiff")));

    let unresolved = TestDir::new("skiff-compiler", "test-only-symbol");
    fs::write(
        unresolved.path().join("package.yml"),
        "id: example.com/test-only-symbol\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        unresolved.path().join("main.skiff"),
        "type Holder { helper: root.test_only.Helper }\n",
    )
    .unwrap();
    fs::write(
        unresolved.path().join("test_only.test.skiff"),
        "type Helper { value: string }\n",
    )
    .unwrap();

    let error = compile_package_project(unresolved.path())
        .expect_err("production source must not resolve test-only symbols")
        .to_string();
    assert!(
        error.contains("invalid root reference"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("root.test_only.Helper"),
        "unexpected error: {error}"
    );
}

fn declared_type_index(value: &serde_json::Value, symbol: &str) -> u64 {
    value["declarations"]["types"][symbol]["typeIndex"]
        .as_u64()
        .unwrap_or_else(|| panic!("missing type index for {symbol}: {value}"))
}

fn assert_json_contains_publication_type(
    value: &serde_json::Value,
    module_path: &str,
    type_index: u64,
) {
    assert!(
        json_contains_publication_type(value, module_path, type_index),
        "missing publication type {module_path}#{type_index}: {value}"
    );
}

fn json_contains_publication_type(
    value: &serde_json::Value,
    module_path: &str,
    type_index: u64,
) -> bool {
    if value["kind"] == "publicationType"
        && value["modulePath"] == module_path
        && value["typeIndex"] == type_index
    {
        return true;
    }
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_contains_publication_type(value, module_path, type_index)),
        serde_json::Value::Object(object) => object
            .values()
            .any(|value| json_contains_publication_type(value, module_path, type_index)),
        _ => false,
    }
}
