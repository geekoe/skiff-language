use std::{collections::BTreeMap, path::PathBuf};

use skiff_artifact_model::{LiteralIr, TypeRefIr};
use skiff_compiler_input::CompilerPlatformSources;

use crate::{
    parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
    publication_db_metadata_index, source_graph::CompilerSourceFile, ExpressionKey,
    ExpressionSourceMap, ExpressionTypeModel, ExpressionTypeModelBuildError,
    PublicationTypeSymbolIndex, TypeResolutionModel,
};

use super::{ObjectFieldValueSource, ObjectMaterializationKind, TargetTypedObjectMaterialization};

const MODULE: &str = "internal.object_materialization";

#[derive(Debug)]
struct BuiltModel {
    source_text: String,
    expression_sources: ExpressionSourceMap,
    model: ExpressionTypeModel,
}

fn build(source_text: &str) -> Result<BuiltModel, ExpressionTypeModelBuildError> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve");
    let platform_sources = CompilerPlatformSources::new(&platform_root)
        .expect("workspace platform sources should load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry should initialize");
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/object_materialization.skiff"),
        MODULE.to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/object_materialization.skiff",
    )
    .expect("object materialization test source should parse");
    let parsed_sources = parse_publication_sources(&PathBuf::from("/test"), &[source])
        .expect("object materialization test source facts should build");
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::new(),
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("object materialization test type resolution should build");
    let expression_sources = ExpressionSourceMap::build(&parsed_sources)
        .expect("object materialization expression source facts should build");
    let db_metadata = publication_db_metadata_index(
        parsed_sources
            .iter()
            .map(|source| (source.module_path(), source.ast())),
        &BTreeMap::new(),
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("object materialization DB metadata should build");
    let model = ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &db_metadata,
        None,
    )?;
    Ok(BuiltModel {
        source_text: source_text.to_string(),
        expression_sources,
        model,
    })
}

impl BuiltModel {
    fn key(&self, snippet: &str) -> ExpressionKey {
        self.expression_sources
            .facts()
            .iter()
            .find_map(|(key, fact)| {
                let source = self
                    .source_text
                    .get(fact.span.start.offset..fact.span.end.offset)?
                    .trim();
                (source == snippet).then_some(key.clone())
            })
            .unwrap_or_else(|| panic!("expression snippet `{snippet}` should have a source fact"))
    }

    fn materialization(&self, snippet: &str) -> &TargetTypedObjectMaterialization {
        let key = self.key(snippet);
        self.model
            .object_materialization(&key)
            .unwrap_or_else(|| panic!("expression snippet `{snippet}` should materialize"))
    }
}

fn field_names(fact: &TargetTypedObjectMaterialization) -> Vec<&str> {
    fact.fields
        .iter()
        .map(|field| field.name.as_str())
        .collect()
}

fn synthetic_fields(fact: &TargetTypedObjectMaterialization) -> Vec<&str> {
    fact.fields
        .iter()
        .filter_map(|field| {
            matches!(field.source, ObjectFieldValueSource::SyntheticNull)
                .then_some(field.name.as_str())
        })
        .collect()
}

fn union_tag(fact: &TargetTypedObjectMaterialization) -> Option<&str> {
    let ObjectMaterializationKind::DiscriminatedUnionBranch { branch } = &fact.kind else {
        return None;
    };
    let TypeRefIr::Record { fields } = &branch.ir else {
        return None;
    };
    match fields.get("tag") {
        Some(TypeRefIr::Literal {
            value: LiteralIr::String { value },
        }) => Some(value),
        _ => None,
    }
}

fn fact_snapshot(fact: &TargetTypedObjectMaterialization) -> String {
    let kind = match &fact.kind {
        ObjectMaterializationKind::Record { construct_target } => {
            format!("record({construct_target})")
        }
        ObjectMaterializationKind::DiscriminatedUnionBranch { .. } => {
            format!("union({})", union_tag(fact).unwrap_or("<unknown>"))
        }
        ObjectMaterializationKind::Map => "map".to_string(),
    };
    let fields = fact
        .fields
        .iter()
        .map(|field| {
            let source = match field.source {
                ObjectFieldValueSource::Provided { .. } => "provided",
                ObjectFieldValueSource::SyntheticNull => "synthetic-null",
            };
            format!("{}:{}={source}", field.name, field.ty)
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "target={};kind={kind};fields=[{fields}]",
        fact.resolved_target
    )
}

#[test]
fn websocket_accept_reject_and_recursive_nominal_facts_snapshot() {
    let built = build(
        r#"
          import std

          function acceptFull() -> std.websocket.WebSocketConnectResult {
            return { tag: "accept", businessIdentity: "biz", connectionPolicy: { maxConnections: 8, overflow: "close-oldest" } }
          }

          function acceptDefaults() -> std.websocket.WebSocketConnectResult {
            return { tag: "accept" }
          }

          function reject() -> std.websocket.WebSocketConnectResult {
            return { tag: "reject", code: 403, reason: "denied" }
          }
        "#,
    )
    .expect("accept and reject object literals should type-check");

    let full = built.materialization(
        r#"{ tag: "accept", businessIdentity: "biz", connectionPolicy: { maxConnections: 8, overflow: "close-oldest" } }"#,
    );
    assert_eq!(union_tag(full), Some("accept"));
    assert_eq!(
        field_names(full),
        [
            "admissionRank",
            "businessIdentity",
            "connectionPolicy",
            "tag"
        ]
    );
    assert_eq!(synthetic_fields(full), ["admissionRank"]);

    let policy = built.materialization(r#"{ maxConnections: 8, overflow: "close-oldest" }"#);
    assert!(matches!(
        policy.kind,
        ObjectMaterializationKind::Record { .. }
    ));
    assert_eq!(
        field_names(policy),
        ["closeCode", "closeReason", "maxConnections", "overflow"]
    );
    assert_eq!(synthetic_fields(policy), ["closeCode", "closeReason"]);

    let defaults = built.materialization(r#"{ tag: "accept" }"#);
    assert_eq!(union_tag(defaults), Some("accept"));
    assert_eq!(
        synthetic_fields(defaults),
        ["admissionRank", "businessIdentity", "connectionPolicy"]
    );
    assert_eq!(
        fact_snapshot(defaults),
        "target=std.websocket.WebSocketConnectResult;kind=union(accept);fields=[admissionRank:integer?=synthetic-null,businessIdentity:string?=synthetic-null,connectionPolicy:std.websocket.WebSocketConnectionPolicy?=synthetic-null,tag:\"accept\"=provided]"
    );
    assert_eq!(
        fact_snapshot(policy),
        "target=std.websocket.WebSocketConnectionPolicy?;kind=record(std.websocket.WebSocketConnectionPolicy);fields=[closeCode:integer?=synthetic-null,closeReason:string?=synthetic-null,maxConnections:integer=provided,overflow:\"close-oldest\" | \"reject-new\"=provided]"
    );

    let reject = built.materialization(r#"{ tag: "reject", code: 403, reason: "denied" }"#);
    assert_eq!(union_tag(reject), Some("reject"));
    assert_eq!(field_names(reject), ["code", "reason", "tag"]);
}

#[test]
fn provided_fields_point_to_recursive_expression_facts() {
    let built = build(
        r#"
          type Context { id: string }
          type Envelope { context: Context, note: string? }

          function envelope() -> Envelope {
            return { context: { id: "ctx" } }
          }
        "#,
    )
    .expect("nested nominal target should type-check");
    let outer = built.materialization(r#"{ context: { id: "ctx" } }"#);
    let context_field = outer
        .fields
        .iter()
        .find(|field| field.name == "context")
        .expect("context field should materialize");
    let ObjectFieldValueSource::Provided { expression } = &context_field.source else {
        panic!("context field should point to its provided expression");
    };
    let nested_key = built.key(r#"{ id: "ctx" }"#);
    assert_eq!(expression, &nested_key);
    assert!(built.model.object_materialization(expression).is_some());
    assert_eq!(synthetic_fields(outer), ["note"]);
}

#[test]
fn map_and_json_targets_keep_map_materialization_facts() {
    let built = build(
        r#"
          type Context { id: string }

          function contexts() -> Map<string, Context> {
            return { beta: { id: "b" }, alpha: { id: "a" } }
          }

          function jsonValue() -> Json {
            return { nested: { enabled: true }, label: "ok" }
          }

          function jsonObject() -> JsonObject {
            return { count: 2 }
          }

          function setNestedJson(body: JsonObject) -> null {
            body.set("reasoning", { effort: "high" })
            return null
          }
        "#,
    )
    .expect("Map/Json object branches should type-check");

    let map = built.materialization(r#"{ beta: { id: "b" }, alpha: { id: "a" } }"#);
    assert!(matches!(map.kind, ObjectMaterializationKind::Map));
    assert_eq!(field_names(map), ["alpha", "beta"]);
    assert_eq!(
        fact_snapshot(map),
        "target=Map<string, Context>;kind=map;fields=[alpha:#0=provided,beta:#0=provided]"
    );
    for nested in [r#"{ id: "a" }"#, r#"{ id: "b" }"#] {
        assert!(matches!(
            built.materialization(nested).kind,
            ObjectMaterializationKind::Record { .. }
        ));
    }

    let json = built.materialization(r#"{ nested: { enabled: true }, label: "ok" }"#);
    assert!(matches!(json.kind, ObjectMaterializationKind::Map));
    assert_eq!(field_names(json), ["label", "nested"]);
    assert!(matches!(
        built.materialization(r#"{ enabled: true }"#).kind,
        ObjectMaterializationKind::Map
    ));
    assert!(matches!(
        built.materialization(r#"{ count: 2 }"#).kind,
        ObjectMaterializationKind::Map
    ));
    assert!(matches!(
        built.materialization(r#"{ effort: "high" }"#).kind,
        ObjectMaterializationKind::Map
    ));
}

#[test]
fn generic_callable_concrete_return_types_plain_bindings_and_object_fields() {
    let built = build(
        r#"
          function encodeJson<T>(value: T) -> Json {
            return std.json.decode<Json>(std.json.encode<T>(value))
          }

          function plainBinding(value: string) -> Json {
            final encoded = encodeJson(value)
            return encoded
          }

          function jsonEnvelope(value: string) -> JsonObject {
            return { event: encodeJson("object-field") }
          }
        "#,
    )
    .expect("generic call with a concrete declared return should retain its expression type");

    for snippet in [
        "encodeJson(value)",
        "encoded",
        r#"encodeJson("object-field")"#,
    ] {
        let key = built.key(snippet);
        let ty = built
            .model
            .fact(&key)
            .and_then(|fact| fact.ty.as_ref())
            .expect("generic call should publish its concrete return type");
        assert!(
            matches!(&ty.ir, TypeRefIr::Builtin { name, args } if name == "Json" && args.is_empty()),
            "expected Json call type, found {:?}",
            ty.ir
        );
    }
}

#[test]
fn generic_callable_type_param_dependent_returns_remain_unresolved() {
    for (source, field) in [
        (
            r#"
              function identity<T>(value: T) -> T { return value }
              function invalid(value: string) -> JsonObject {
                return { direct: identity(value) }
              }
            "#,
            "direct",
        ),
        (
            r#"
              function singleton<T>(value: T) -> Array<T> {
                final items = Array.empty<T>()
                items.push(value)
                return items
              }
              function invalid(value: string) -> JsonObject {
                return { nested: singleton(value) }
              }
            "#,
            "nested",
        ),
    ] {
        let error = build(source)
            .expect_err("type-param-dependent call return must remain unresolved")
            .message();
        assert!(
            error.contains(&format!(
                "object literal field `{field}` has no resolved expression type"
            )),
            "unexpected dependent-return diagnostic:\n{error}"
        );
    }
}

#[test]
fn nested_object_targets_reject_missing_extra_and_incompatible_fields() {
    for (source, expected) in [
        (
            r#"
              type Details { enabled: bool }
              type Envelope { details: Details }
              function invalid() -> Envelope {
                return { details: {} }
              }
            "#,
            "object literal field `details` type mismatch",
        ),
        (
            r#"
              type Details { enabled: bool }
              type Envelope { details: Details }
              function invalid() -> Envelope {
                return { details: { enabled: true, extra: "no" } }
              }
            "#,
            "object literal field `details` type mismatch",
        ),
        (
            r#"
              type Details { enabled: bool }
              type Envelope { details: Details }
              function invalid() -> Envelope {
                return { details: { enabled: "no" } }
              }
            "#,
            "object literal field `details` type mismatch",
        ),
    ] {
        let error = build(source).expect_err("invalid nested object must be rejected");
        assert!(
            error
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains(expected)),
            "expected `{expected}` in {:?}",
            error.diagnostics
        );
    }
}

#[test]
fn stream_emit_materializes_record_union_nullable_and_nested_object_facts() {
    let built = build(
        r#"
          type Details { label: string, note: string? }
          type RecordChunk { sequence: integer, details: Details, optional: string? }
          alias UnionChunk =
            { tag: "record", payload: RecordChunk, trace: string? }
            | { tag: "done", count: integer }

          function recordEvents() -> Stream<RecordChunk> {
            emit({ sequence: 1, details: { label: "record-details" } })
            return
          }

          function unionEvents() -> Stream<UnionChunk> {
            emit({ tag: "record", payload: { sequence: 2, details: { label: "union-details" } } })
            emit({ tag: "done", count: 2 })
            return
          }
        "#,
    )
    .expect("stream emit targets should drive recursive object materialization");

    let record_snippet = r#"{ sequence: 1, details: { label: "record-details" } }"#;
    let record_key = built.key(record_snippet);
    let record = built.materialization(record_snippet);
    assert!(matches!(
        record.kind,
        ObjectMaterializationKind::Record { .. }
    ));
    assert_eq!(field_names(record), ["details", "optional", "sequence"]);
    assert_eq!(synthetic_fields(record), ["optional"]);
    assert_eq!(
        built
            .model
            .stream_emit_target(&record_key)
            .expect("record emit should persist its target")
            .ir,
        record.resolved_target.ir
    );

    let record_details = built.materialization(r#"{ label: "record-details" }"#);
    assert_eq!(field_names(record_details), ["label", "note"]);
    assert_eq!(synthetic_fields(record_details), ["note"]);

    let union_snippet =
        r#"{ tag: "record", payload: { sequence: 2, details: { label: "union-details" } } }"#;
    let union_key = built.key(union_snippet);
    let union = built.materialization(union_snippet);
    assert_eq!(union_tag(union), Some("record"));
    assert_eq!(field_names(union), ["payload", "tag", "trace"]);
    assert_eq!(synthetic_fields(union), ["trace"]);
    assert_eq!(
        built
            .model
            .stream_emit_target(&union_key)
            .expect("union emit should persist its target")
            .ir,
        union.resolved_target.ir
    );

    let union_payload =
        built.materialization(r#"{ sequence: 2, details: { label: "union-details" } }"#);
    assert!(matches!(
        union_payload.kind,
        ObjectMaterializationKind::Record { .. }
    ));
    assert_eq!(synthetic_fields(union_payload), ["optional"]);
    assert_eq!(
        synthetic_fields(built.materialization(r#"{ label: "union-details" }"#)),
        ["note"]
    );

    let done_snippet = r#"{ tag: "done", count: 2 }"#;
    assert_eq!(union_tag(built.materialization(done_snippet)), Some("done"));
    assert!(built
        .model
        .stream_emit_target(&built.key(done_snippet))
        .is_some());
}

#[test]
fn exact_http_response_start_emit_contextualizes_direct_empty_headers() {
    let built = build(
        r#"
          import std

          function events() -> Stream<std.http.HttpResponseStreamEvent> {
            emit({ tag: "start", status: 207, headers: [] })
            emit({ tag: "end" })
            return
          }
        "#,
    )
    .expect("exact response start Emit should contextually type direct empty headers");

    let headers_key = built.key("[]");
    let headers = built
        .model
        .fact(&headers_key)
        .and_then(|fact| fact.ty.as_ref())
        .expect("empty headers should retain a concrete source type fact");
    let TypeRefIr::Builtin { name, args } = &headers.ir else {
        panic!(
            "empty headers should be an exact Array, found {:?}",
            headers.ir
        )
    };
    assert_eq!(name, "Array");
    let [TypeRefIr::PackageSymbol { symbol }] = args.as_slice() else {
        panic!("empty headers should carry the exact HttpHeader item: {args:?}")
    };
    assert_eq!(
        symbol.package,
        skiff_artifact_model::PackageRefIr::PackageId {
            package_id: "skiff.run/std".to_string()
        }
    );
    assert_eq!(symbol.symbol_path, "std.http.HttpHeader");

    let start = built.materialization(r#"{ tag: "start", status: 207, headers: [] }"#);
    assert_eq!(union_tag(start), Some("start"));
    assert_eq!(
        start
            .fields
            .iter()
            .find(|field| field.name == "headers")
            .expect("start branch has headers")
            .ty
            .ir,
        headers.ir
    );
}

#[test]
fn response_start_empty_headers_context_does_not_escape_exact_direct_emit() {
    let cases = [
        (
            r#"
              import std
              type Lookalike discriminator "tag" =
                { tag: "start", status: integer, headers: Array<std.http.HttpHeader> }
                | { tag: "end" }

              function events() -> Stream<Lookalike> {
                emit({ tag: "start", status: 207, headers: [] })
                return
              }
            "#,
            "object literal field `headers` type mismatch",
        ),
        (
            r#"
              import std
              function value() -> std.http.HttpResponseStreamEvent {
                return { tag: "start", status: 207, headers: [] }
              }
            "#,
            "object literal field `headers` type mismatch",
        ),
        (
            r#"
              import std
              function events() -> Stream<std.http.HttpResponseStreamEvent> {
                final headers = []
                emit({ tag: "start", status: 207, headers: headers })
                return
              }
            "#,
            "object literal field `headers` type mismatch",
        ),
        (
            r#"
              import std
              function events() -> Stream<std.http.HttpResponseStreamEvent> {
                emit({ tag: "chunk", value: [] })
                return
              }
            "#,
            "object literal field `value` type mismatch",
        ),
        (
            r#"
              import std
              function events() -> Stream<std.http.HttpResponseStreamEvent> {
                emit({ tag: "start", status: 207, headers: Map.empty<string, string>() })
                return
              }
            "#,
            "object literal field `headers` type mismatch",
        ),
    ];

    for (source, expected) in cases {
        let error = build(source)
            .expect_err("contextual empty-array authority must remain exact and direct")
            .message();
        assert!(
            error.contains(expected),
            "expected diagnostic {expected:?}, got:\n{error}"
        );
    }
}

#[test]
fn stream_emit_object_and_scalar_negatives_fail_in_the_unified_type_owner() {
    let cases = [
        (
            r#"
              type Chunk { required: string, nullable: string? }
              function events() -> Stream<Chunk> { emit({}) return }
            "#,
            "missing required object literal field `required`",
        ),
        (
            r#"
              type Chunk { value: string }
              function events() -> Stream<Chunk> {
                emit({ value: "ok", extra: true })
                return
              }
            "#,
            "unknown object literal field `extra`",
        ),
        (
            r#"
              alias Chunk = { left: string? } | { right: string? }
              function events() -> Stream<Chunk> { emit({}) return }
            "#,
            "ambiguous object literal branch",
        ),
        (
            r#"
              function events() -> Stream<string> { emit(42) return }
            "#,
            "emit chunk type mismatch",
        ),
    ];

    for (source, expected) in cases {
        let error = build(source)
            .expect_err("invalid stream emit should fail source typing")
            .message();
        assert!(
            error.contains(expected),
            "expected diagnostic {expected:?}, got:\n{error}"
        );
    }
}

#[test]
fn object_literal_negatives_fail_closed() {
    let cases = [
        (
            r#"
              alias Choice = { left: string? } | { right: string? }
              function ambiguous() -> Choice { return {} }
            "#,
            "ambiguous object literal branch",
        ),
        (
            r#"
              type Context { id: string }
              function extra() -> Context { return { id: "ctx", extra: true } }
            "#,
            "unknown object literal field `extra`",
        ),
        (
            r#"
              type Context { id: string }
              function missing() -> Context { return {} }
            "#,
            "missing required object literal field `id`",
        ),
    ];

    for (source, expected) in cases {
        let error = build(source)
            .expect_err("invalid target-typed object literal should fail")
            .message();
        assert!(
            error.contains(expected),
            "expected diagnostic {expected:?}, got:\n{error}"
        );
    }
}
