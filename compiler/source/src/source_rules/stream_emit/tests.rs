use super::*;
use std::{collections::BTreeMap, path::PathBuf};

use skiff_compiler_input::CompilerPlatformSources;

use crate::{
    parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
    publication_db_metadata_index, source_graph::CompilerSourceFile, ExpressionSourceMap,
    PublicationTypeSymbolIndex, TypeResolutionModel,
};

fn collect(source: &str) -> Vec<String> {
    let ast = crate::shared::parser::parse_source(source).unwrap();
    let mut return_types = BTreeMap::new();
    collect_stream_function_return_types(&ast, &mut return_types);
    let mut violations = Vec::new();
    collect_stream_emit_violations("test.skiff", &ast, &return_types, &mut violations);
    violations
}

fn collect_typed(source: &str) -> Vec<String> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve");
    let platform_sources = CompilerPlatformSources::new(&platform_root)
        .expect("workspace platform sources should load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry should initialize");
    let source = CompilerSourceFile::parse(
        PathBuf::from("stream_emit.skiff"),
        "stream_emit".to_string(),
        false,
        false,
        source.to_string(),
        "stream_emit.skiff",
    )
    .expect("stream emit test source should parse");
    let diagnostic_root = PathBuf::from("/test");
    let parsed_sources = parse_publication_sources(&diagnostic_root, &[source])
        .expect("stream emit parsed source facts should build");
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::new(),
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("stream emit type resolution should build");
    let expression_sources = ExpressionSourceMap::build(&parsed_sources)
        .expect("stream emit expression source facts should build");
    let db_metadata = publication_db_metadata_index(
        parsed_sources
            .iter()
            .map(|source| (source.module_path(), source.ast())),
        &BTreeMap::new(),
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("stream emit DB metadata should build");
    let expression_types = ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &db_metadata,
        None,
    )
    .expect("stream emit expression type facts should build");
    let mut violations = Vec::new();
    collect_stream_emit_type_violations(
        &diagnostic_root,
        &parsed_sources,
        &expression_types,
        &mut violations,
    );
    violations
}

#[test]
fn rejects_emit_expression_call() {
    let violations = collect(
        r#"
                type Chunk {}

                function makeChunk() -> Chunk {
                    return {}
                }

                function events() -> Stream<Chunk> {
                    const ignored = emit(makeChunk())
                    return {}
                }
            "#,
    );

    assert_eq!(
        violations,
        vec!["test.skiff: emit is a stream statement and cannot be used as an expression"]
    );
}

#[test]
fn rejects_emit_in_non_stream_function() {
    let violations = collect(
        r#"
                type Chunk {}

                function echo(chunk: Chunk) -> Chunk {
                    emit(chunk)
                    return chunk
                }
            "#,
    );

    assert_eq!(
            violations,
            vec![
                "test.skiff: emit can only be used in a Stream<T> producer; function echo returns a non-stream type"
            ]
        );
}

#[test]
fn allows_emit_inside_while_loop_body() {
    let violations = collect_typed(
        r#"
                type Chunk { value: string }

                function events() -> Stream<Chunk> {
                  while true {
                    emit({ value: "ok" })
                    break
                  }
                  return
                }
            "#,
    );

    assert_eq!(violations, Vec::<String>::new());
}

#[test]
fn typed_checker_consumes_emit_facts_and_only_reports_stream_control_violations() {
    let violations = collect_typed(
        r#"
                type Chunk { value: string }

                function valid() -> Stream<Chunk> {
                    emit({ value: "ok" })
                    return
                }

                function invalid() -> Chunk {
                    emit(Chunk { value: "invalid" })
                    return Chunk { value: "return" }
                }
            "#,
    );

    assert_eq!(violations.len(), 1, "violations: {violations:#?}");
    assert!(violations[0].contains(
        "emit can only be used in a Stream<T> producer; function invalid returns a non-stream type"
    ));
}

#[test]
fn rejects_local_annotation_and_chunk_type_mismatch() {
    let violations = collect(
        r#"
                type GoodChunk {}
                type BadChunk {}

                function makeWrongChunk() -> BadChunk {
                    return {}
                }

                function events() -> Stream<GoodChunk> {
                    const chunk: GoodChunk = makeWrongChunk()
                    emit(chunk)
                    return {}
                }
            "#,
    );

    assert_eq!(
            violations,
            vec![
                "test.skiff: local binding chunk annotation type mismatch in events: expected GoodChunk, found BadChunk",
                "test.skiff: emit chunk type mismatch in events: expected GoodChunk, found BadChunk",
            ]
        );
}
