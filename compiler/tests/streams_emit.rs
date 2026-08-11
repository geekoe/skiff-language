mod common;

use std::fs;

use common::{artifacts::module_artifact, package_project::compile_package_project, TestDir};
use serde_json::Value;
use skiff_artifact_model::PackageLocalAbiSymbol;
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;

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

fn count_json_nodes(value: &Value, predicate: impl Fn(&Value) -> bool + Copy) -> usize {
    usize::from(predicate(value))
        + match value {
            Value::Object(object) => object
                .values()
                .map(|child| count_json_nodes(child, predicate))
                .sum(),
            Value::Array(items) => items
                .iter()
                .map(|child| count_json_nodes(child, predicate))
                .sum(),
            _ => 0,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_file_ir_contains_for_emit_and_std_sse_package_call() {
        let temp = TestDir::new("skiff-compiler", "stream-file-ir");
        fs::write(
            temp.path().join("package.yml"),
            "id: example.com/stream-file-ir\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("stream.skiff"),
            r#"
import std

function events(request: std.http.HttpClientRequest) -> Stream<std.http.HttpSseEvent> {
  for event in std.http.sse(request) {
    emit(event)
  }
  return
}
"#,
        )
        .unwrap();

        let project = compile_package_project(temp.path()).expect("stream package should compile");
        let value = module_artifact(&project.package, "stream").value();
        let std = project
            .dependency(SKIFF_STD_PUBLICATION_ID, "1.0.0")
            .expect("std should be in the canonical dependency closure");
        let Some(PackageLocalAbiSymbol::Callable {
            callable_id: sse_callable_id,
            ..
        }) = std
            .artifact
            .package_local_abi
            .public_symbols
            .get("std.http.sse")
        else {
            panic!("std should expose std.http.sse");
        };

        assert!(find_json_node(&value, |node| node["kind"] == "forIn").is_some());
        assert!(find_json_node(&value, |node| node["kind"] == "emit").is_some());
        assert!(find_json_node(&value, |node| {
            node["kind"] == "call"
                && node["call"]["target"]["kind"] == "packageCallable"
                && node["call"]["target"]["packageRef"]["kind"] == "dependency"
                && node["call"]["target"]["packageRef"]["dependencyRef"] == "std"
                && node["call"]["target"]["packageCallableId"] == sse_callable_id.as_str()
        })
        .is_some());
    }

    #[test]
    fn target_typed_record_chunks_compile_to_package_file_ir() {
        let temp = TestDir::new("skiff-compiler", "target-typed-stream-chunks");
        fs::write(
            temp.path().join("package.yml"),
            "id: example.com/target-typed-stream-chunks\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("stream.skiff"),
            r#"
type Profile { nickname: string }
type GoodChunk { value: string, profile: Profile }

function flatEvents() -> Stream<GoodChunk> {
  emit({ value: "ok", profile: { nickname: "Ada" } })
  return
}

function nestedEvents() -> Stream<GoodChunk> {
  final chunk: GoodChunk = { value: "ok", profile: { nickname: "Grace" } }
  emit(chunk)
  return
}
"#,
        )
        .unwrap();

        let project = compile_package_project(temp.path()).expect("typed chunks should compile");
        let value = module_artifact(&project.package, "stream").value();
        let emit_count = count_json_nodes(&value, |node| node["kind"] == "emit");
        assert_eq!(emit_count, 2);
    }
}
