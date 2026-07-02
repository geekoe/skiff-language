mod common;
use common::artifacts::{assert_publish_error_contains, build_temp_service_publication};
use serde_json::Value;
use skiff_compiler::test_support::project_fixtures::ServiceProjectBuilder;

fn find_json_node<'a>(
    value: &'a Value,
    predicate: impl Fn(&'a Value) -> bool + Copy,
) -> Option<&'a Value> {
    if predicate(value) {
        return Some(value);
    }
    match value {
        Value::Object(object) => object
            .values()
            .find_map(|child| find_json_node(child, predicate)),
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_json_node(child, predicate)),
        _ => None,
    }
}

#[test]
fn request_local_stream_producer_sse_for_loop_and_emit_summary_publish() {
    let temp = ServiceProjectBuilder::package_model(
        "request-local-stream-producer",
        "import std",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/stream_helper.skiff",
        r#"
            import std

            function events(request: std.http.HttpClientRequest) -> Stream<std.http.HttpSseEvent> {
              for event in std.http.sse(request) {
                emit(event)
              }
              return
            }
        "#,
    );
    rewrite_service_config_without_packages(&temp);

    let published = build_temp_service_publication(temp.root());
    let stream_helper = published
        .artifacts
        .file_ir_units
        .iter()
        .find(|artifact| artifact.source_path == "internal/stream_helper.skiff")
        .expect("stream helper File IR unit should be published");
    let stream_helper_value = stream_helper.value();

    assert!(find_json_node(&stream_helper_value, |node| node["kind"] == "forIn").is_some());
    assert!(find_json_node(&stream_helper_value, |node| node["kind"] == "emit").is_some());
    assert!(find_json_node(&stream_helper_value, |node| {
        node["kind"] == "call"
            && node["call"]["target"]["kind"] == "native"
            && node["call"]["target"]["target"]["namespace"] == "std.http"
            && node["call"]["target"]["target"]["symbol"] == "sse"
    })
    .is_some());
}

#[test]
fn emit_in_non_stream_function_is_rejected() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-non-stream",
        "",
        r#"
          emit(input)
          return {}
        "#,
    );

    assert_publish_error_contains(temp.root(), &["emit", "Stream<T> producer"]);
}

#[test]
fn mismatched_emit_chunk_type_is_rejected() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-mismatched-chunk",
        "import std",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/stream_helper.skiff",
        r#"
            import std

            function events(request: std.http.HttpClientRequest) -> Stream<std.http.HttpClientResponse> {
              for event in std.http.sse(request) {
                emit(event)
              }
              return
            }
        "#,
    );
    rewrite_service_config_without_packages(&temp);

    assert_publish_error_contains(
        temp.root(),
        &[
            "emit chunk type mismatch",
            "std.http.HttpClientResponse",
            "HttpSseEvent",
        ],
    );
}

#[test]
fn emit_allows_target_typed_object_literal_chunk() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-target-typed-object-literal-chunk",
        "",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/stream_helper.skiff",
        r#"
            type GoodChunk {
              value: string
            }

            function events() -> Stream<GoodChunk> {
              emit({ value: "ok" })
              return
            }
        "#,
    );

    build_temp_service_publication(temp.root());
}

#[test]
fn emit_allows_nested_target_typed_object_literal_chunk() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-nested-target-typed-object-literal-chunk",
        "",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/stream_helper.skiff",
        r#"
            type Profile {
              nickname: string
            }
            type GoodChunk {
              profile: Profile
            }

            function events() -> Stream<GoodChunk> {
              emit({ profile: { nickname: "Ada" } })
              return
            }
        "#,
    );

    build_temp_service_publication(temp.root());
}

#[test]
fn emit_rejects_invalid_nested_target_typed_object_literal_chunk_by_field() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-invalid-nested-target-typed-object-literal-chunk",
        "",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/stream_helper.skiff",
        r#"
            type Profile {
              nickname: string
            }
            type GoodChunk {
              profile: Profile
            }

            function events() -> Stream<GoodChunk> {
              emit({ profile: {} })
              return
            }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &[
            "emit chunk in events object literal field `profile` type mismatch",
            "Profile",
        ],
    );
}

#[test]
fn emit_rejects_project_function_returning_wrong_chunk_type() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-project-function-wrong-chunk",
        "",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/stream_helper.skiff",
        r#"
            type GoodChunk {
              value: string
            }
            type BadChunk {}

            function makeWrongChunk() -> BadChunk {
              return {}
            }

            function events() -> Stream<GoodChunk> {
              emit(makeWrongChunk())
              return
            }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &["emit chunk type mismatch", "GoodChunk", "BadChunk"],
    );
}

fn rewrite_service_config_without_packages(temp: &ServiceProjectBuilder) {
    temp.add_root_file(
        "service.yml",
        r#"
id: example.com/example
version: 1.0.0
"#,
    );
}

#[test]
fn emit_rejects_local_const_bound_to_wrong_chunk_type() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-local-const-wrong-chunk",
        "",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/stream_helper.skiff",
        r#"
            type GoodChunk {}
            type BadChunk {}

            function makeWrongChunk() -> BadChunk {
              return {}
            }

            function events() -> Stream<GoodChunk> {
              const chunk = makeWrongChunk()
              emit(chunk)
              return
            }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &["emit chunk type mismatch", "GoodChunk", "BadChunk"],
    );
}

#[test]
fn emit_rejects_object_db_write_result_as_nominal_chunk() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-object-db-write-result",
        "",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/db_stream.skiff",
        r#"
            type User {
              id: string,
              name: string,
              visits: number
            }

            db object User {
              name "user"
              primary key(id)
            }

            function events() -> Stream<User> {
              emit(db update User("u1") { visits += 1 })
              return
            }
        "#,
    );

    assert_publish_error_contains(temp.root(), &["emit chunk type mismatch", "User", "User?"]);
}

#[test]
fn emit_rejects_local_const_annotation_hiding_wrong_initializer_type() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-local-const-annotation-hides-wrong-chunk",
        "",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/stream_helper.skiff",
        r#"
            type GoodChunk {}
            type BadChunk {}

            function makeWrongChunk() -> BadChunk {
              return {}
            }

            function events() -> Stream<GoodChunk> {
              const chunk: GoodChunk = makeWrongChunk()
              emit(chunk)
              return
            }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &["annotation type mismatch", "GoodChunk", "BadChunk"],
    );
}

#[test]
fn emit_rejects_unknown_initializer_hidden_by_local_const_annotation() {
    let temp = ServiceProjectBuilder::package_model(
        "emit-local-const-unknown-initializer-annotation",
        "",
        r#"
          return {}
        "#,
    );
    temp.add_source(
        "internal/stream_helper.skiff",
        r#"
            type GoodChunk {
              value: string
            }

            function events() -> Stream<GoodChunk> {
              const chunk: GoodChunk = {}
              emit(chunk)
              return
            }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &[
            "local binding chunk annotation missing required object literal field `value`",
            "GoodChunk",
        ],
    );
}
