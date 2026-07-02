mod common;

use common::artifacts::{
    assert_publish_error_contains, build_temp_service_publication, source_artifact,
};
use skiff_compiler::test_support::project_fixtures::ServiceProjectBuilder;

#[test]
fn root_path_resolves_internal_module_type_reference() {
    let temp = ServiceProjectBuilder::package_model_with_internal_module(
        "type-ref",
        "helpers",
        "type Helper { value: String }\n",
        "",
        "let _h: root.internal.helpers.Helper = root.internal.helpers.Helper { value: \"hi\" }\n                return Output {}",
    );
    let published = build_temp_service_publication(temp.root());
    assert!(!published.artifacts.file_ir_units.is_empty());
    let artifact = source_artifact(&published, "internal/example.skiff");
    let artifact_value = artifact.value();
    assert!(artifact_value["sourceAstHash"]
        .as_str()
        .unwrap()
        .starts_with("skiff-source-ast-v1:sha256:"));
    let helper_artifact = source_artifact(&published, "internal/helpers.skiff");
    let helper_type_index = declared_type_index(&helper_artifact.value(), "Helper");
    assert_json_contains_publication_type(&artifact_value, "internal.helpers", helper_type_index);
    assert!(!json_contains_service_symbol(
        &artifact_value,
        "internal.helpers",
        "Helper"
    ));
    assert!(!artifact_value
        .to_string()
        .contains("root.internal.helpers.Helper"));
}

#[test]
fn root_path_canonical_type_beats_local_same_name() {
    let temp = ServiceProjectBuilder::package_model_with_internal_module(
        "same-name",
        "helpers",
        "type Helper { value: String }\n",
        "type Helper { local: String }",
        "let _h: root.internal.helpers.Helper = root.internal.helpers.Helper { value: \"hi\" }\n                return Output {}",
    );
    let published = build_temp_service_publication(temp.root());
    let artifact = source_artifact(&published, "internal/example.skiff");
    let artifact_value = artifact.value();
    let helper_artifact = source_artifact(&published, "internal/helpers.skiff");
    let helper_type_index = declared_type_index(&helper_artifact.value(), "Helper");
    assert_json_contains_publication_type(&artifact_value, "internal.helpers", helper_type_index);
    assert!(!json_contains_service_symbol(
        &artifact_value,
        "internal.helpers",
        "Helper"
    ));
    assert!(!artifact_value
        .to_string()
        .contains("root.internal.helpers.Helper"));
}

#[test]
fn root_path_resolves_internal_attached_db_object_as_type_reference() {
    let temp = ServiceProjectBuilder::package_model_with_internal_module(
        "db-object-type-ref",
        "models",
        r#"
            type Thread {
              id: string,
              ownerUserId: string
            }

            db object Thread {
              name "thread"
              primary key(id)
            }
        "#,
        r#"
            type Holder {
              thread: root.internal.models.Thread
            }
        "#,
        "return Output {}",
    );
    let published = build_temp_service_publication(temp.root());

    let model_artifact = source_artifact(&published, "internal/models.skiff");
    let model_value = model_artifact.value();
    assert!(!model_value["declarations"]["types"]["Thread"].is_null());
    assert_eq!(
        model_value["declarations"]["db"]["Thread"]["typeRef"],
        serde_json::json!({
            "kind": "dbObjectSymbol",
            "symbol": { "modulePath": "internal.models", "symbol": "Thread" }
        })
    );

    let consumer_artifact = source_artifact(&published, "internal/example.skiff");
    let consumer_value = consumer_artifact.value();
    let thread_type_index = declared_type_index(&model_value, "Thread");
    assert_json_contains_publication_type(&consumer_value, "internal.models", thread_type_index);
    assert!(!json_contains_service_symbol(
        &consumer_value,
        "internal.models",
        "Thread"
    ));
    assert!(!consumer_value
        .to_string()
        .contains("root.internal.models.Thread"));
}

#[test]
fn root_path_implements_reference_links() {
    let temp = ServiceProjectBuilder::new("root-path-implements")
        .write_root_file(
            "service.yml",
            r#"
id: example.com/example
version: 1.0.0
"#,
        )
        .write_root_file(
            "api.yml",
            r#"
Interface: internal.impl.Interface
api:
  http:
    Input: api.http.Input
    Output: api.http.Output
    Interface: api.http.Interface
"#,
        )
        .write_source(
            "api/http.skiff",
            r#"
type Input {}
type Output {}
interface Interface {
  function run(input: Input) -> Output
}
"#,
        )
        .write_source(
            "internal/impl.skiff",
            r#"
type Interface {}

impl Interface {
  function run(self: Interface, input: root.api.http.Input) -> root.api.http.Output {
    return root.api.http.Output {}
  }
}
"#,
        );
    let published = build_temp_service_publication(temp.root());
    assert_eq!(
        published.artifacts.service_unit.value["operations"][0]["executable"]["fileRef"]
            ["modulePath"],
        "internal.impl"
    );
    assert!(!published
        .artifacts
        .service_unit
        .value
        .to_string()
        .contains("root.api.http.Interface"));
}

#[test]
fn root_path_actor_ref_receiver_call_is_rejected() {
    let temp = ServiceProjectBuilder::new("root-path-actor-return")
        .write_root_file(
            "service.yml",
            r#"
id: example.com/example
version: 1.0.0
"#,
        )
        .write_root_file(
            "api.yml",
            r#"
ExampleService: internal.example.ExampleService
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
            "internal/thread_coordinator.skiff",
            r#"
type ThreadCoordinator implements std.actor.Actor<string> {
  threadId: string
}

function passthrough(actor: ActorRef<ThreadCoordinator>) -> ActorRef<ThreadCoordinator> {
  return actor
}

impl ThreadCoordinator {
  function receiveUserMessage(self: ThreadCoordinator, content: string) -> string {
    return content
  }
}
"#,
        )
        .write_source(
            "internal/example.skiff",
            r#"
type ExampleService {}

function send(actor: ActorRef<root.internal.thread_coordinator.ThreadCoordinator>) -> string {
  const co = root.internal.thread_coordinator.passthrough(actor)
  return co.receiveUserMessage("hello")
}

impl ExampleService {
  function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
    return root.api.example.Output {}
  }
}
"#,
        );
    assert_publish_error_contains(
        temp.root(),
        &["ActorRef receiver method calls are no longer supported"],
    );
}

#[test]
fn root_path_unknown_module_fails_publish() {
    let temp = ServiceProjectBuilder::package_model_with_internal_module(
        "unknown-module",
        "helpers",
        "type Helper { value: String }\n",
        "",
        "let _h: root.internal.missing.Helper = Helper { value: \"hi\" }\n                return Output {}",
    );
    assert_publish_error_contains(
        temp.root(),
        &[
            "invalid root reference",
            "root.internal.missing.Helper",
            "internal/missing.skiff",
        ],
    );
}

#[test]
fn root_path_unknown_symbol_fails_publish() {
    let temp = ServiceProjectBuilder::package_model_with_internal_module(
        "unknown-symbol",
        "helpers",
        "type Helper { value: String }\n",
        "",
        "let _h: root.internal.helpers.Missing = Helper { value: \"hi\" }\n                return Output {}",
    );
    assert_publish_error_contains(
        temp.root(),
        &[
            "invalid root reference",
            "root.internal.helpers.Missing",
            "Missing",
        ],
    );
}

#[test]
fn test_file_root_path_errors_do_not_affect_production_publish() {
    let temp = ServiceProjectBuilder::package_model_with_internal_module(
        "test-root-reference-ignored",
        "helpers",
        "type Helper { value: String }\n",
        "",
        "return Output {}",
    )
    .write_source(
        "internal/example.test.skiff",
        r#"
            test "test-only root reference" {
              let _missing: root.internal.missing.Helper = root.internal.missing.Helper { value: "hi" }
              assert true
            }
        "#,
    );

    let published = build_temp_service_publication(temp.root());

    assert!(published
        .artifacts
        .file_ir_units
        .iter()
        .all(|artifact| !artifact.source_path.ends_with(".test.skiff")));
}

#[test]
fn production_root_path_does_not_resolve_test_only_symbols() {
    let temp = ServiceProjectBuilder::package_model_with_internal_module(
        "production-root-reference-test-only-symbol",
        "helpers",
        "type Helper { value: String }\n",
        "",
        r#"
          let _helper: root.internal.test_only.Helper = root.internal.test_only.Helper { value: "hi" }
          return Output {}
        "#,
    )
    .write_source(
        "internal/test_only.test.skiff",
        r#"
            type Helper { value: string }

            test "test-only helper" {
              assert true
            }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &[
            "invalid root reference",
            "root.internal.test_only.Helper",
            "internal/test_only.skiff",
        ],
    );
}

#[test]
fn root_path_resolves_unexported_internal_symbol() {
    let temp = ServiceProjectBuilder::package_model_with_internal_module(
        "unexported-symbol",
        "helpers",
        "type Helper { value: String }\n",
        "",
        "let _h: root.internal.helpers.Helper = root.internal.helpers.Helper { value: \"hi\" }\n                return Output {}",
    );
    let published = build_temp_service_publication(temp.root());
    let artifact = source_artifact(&published, "internal/example.skiff");
    let helper_artifact = source_artifact(&published, "internal/helpers.skiff");
    let helper_type_index = declared_type_index(&helper_artifact.value(), "Helper");
    assert_json_contains_publication_type(&artifact.value(), "internal.helpers", helper_type_index);
    assert!(!json_contains_service_symbol(
        &artifact.value(),
        "internal.helpers",
        "Helper"
    ));
}

#[test]
fn publication_direct_refs_do_not_leak_into_public_projection() {
    let temp = ServiceProjectBuilder::new("publication-direct-ref-public-projection")
        .write_root_file(
            "service.yml",
            r#"
id: example.com/example
version: 1.0.0
"#,
        )
        .write_root_file(
            "api.yml",
            r#"
ExampleService: internal.runner.ExampleService
api:
  example:
    Input: internal.types.Payload
    Output: internal.types.Payload
    ExampleService: api.example.ExampleService
"#,
        )
        .write_source(
            "api/example.skiff",
            r#"
interface ExampleService {
  function run(input: root.internal.types.Payload) -> root.internal.types.Payload
}
"#,
        )
        .write_source(
            "internal/types.skiff",
            r#"
type Payload {
  value: string
}
"#,
        )
        .write_source(
            "internal/runner.skiff",
            r#"
type ExampleService {}

impl ExampleService {
  function run(self: ExampleService, input: root.internal.types.Payload) -> root.internal.types.Payload {
    return input
  }
}
"#,
        );

    let published = build_temp_service_publication(temp.root());
    let runner_artifact = source_artifact(&published, "internal/runner.skiff");
    let types_artifact = source_artifact(&published, "internal/types.skiff");
    let payload_type_index = declared_type_index(&types_artifact.value(), "Payload");
    assert_json_contains_publication_type(
        &runner_artifact.value(),
        "internal.types",
        payload_type_index,
    );

    let publication_abi = &published.artifacts.service_unit.value["publicationAbi"];
    let publication_abi_text = publication_abi.to_string();
    assert!(
        !publication_abi_text.contains("publicationType"),
        "publication ABI leaked publicationType: {publication_abi}"
    );
    assert!(
        !publication_abi_text.contains("$type"),
        "publication ABI leaked raw type placeholder: {publication_abi}"
    );
    assert!(
        !publication_abi_text.contains("__unresolved_publication_type"),
        "publication ABI leaked unresolved placeholder: {publication_abi}"
    );
    assert!(
        json_contains_service_symbol(publication_abi, "internal.types", "Payload"),
        "publication ABI should use stable source symbol identity: {publication_abi}"
    );

    let service_assembly_text = published.artifacts.service_assembly.value.to_string();
    assert!(!service_assembly_text.contains("$type"));
    assert!(!service_assembly_text.contains("__unresolved_publication_type"));
}

fn declared_type_index(value: &serde_json::Value, symbol: &str) -> u64 {
    value["declarations"]["types"][symbol]["typeIndex"]
        .as_u64()
        .unwrap_or_else(|| panic!("missing type declaration index for {symbol}: {value}"))
}

fn assert_json_contains_publication_type(
    value: &serde_json::Value,
    module_path: &str,
    type_index: u64,
) {
    assert!(
        json_contains_publication_type(value, module_path, type_index),
        "expected publication type {module_path}#{type_index}: {value}"
    );
}

fn json_contains_publication_type(
    value: &serde_json::Value,
    module_path: &str,
    type_index: u64,
) -> bool {
    if value.get("kind").and_then(serde_json::Value::as_str) == Some("publicationType")
        && value.get("modulePath").and_then(serde_json::Value::as_str) == Some(module_path)
        && value.get("typeIndex").and_then(serde_json::Value::as_u64) == Some(type_index)
    {
        return true;
    }

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_contains_publication_type(item, module_path, type_index)),
        serde_json::Value::Object(object) => object
            .values()
            .any(|item| json_contains_publication_type(item, module_path, type_index)),
        _ => false,
    }
}

fn json_contains_service_symbol(
    value: &serde_json::Value,
    module_path: &str,
    symbol: &str,
) -> bool {
    if value.get("kind").and_then(serde_json::Value::as_str) == Some("serviceSymbol")
        && value
            .get("symbol")
            .and_then(|symbol| symbol.get("modulePath"))
            .and_then(serde_json::Value::as_str)
            == Some(module_path)
        && value
            .get("symbol")
            .and_then(|symbol| symbol.get("symbol"))
            .and_then(serde_json::Value::as_str)
            == Some(symbol)
    {
        return true;
    }

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_contains_service_symbol(item, module_path, symbol)),
        serde_json::Value::Object(object) => object
            .values()
            .any(|item| json_contains_service_symbol(item, module_path, symbol)),
        _ => false,
    }
}
