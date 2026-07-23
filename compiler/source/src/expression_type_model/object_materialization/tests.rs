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
            format!("record({})", construct_target.source_text)
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
            format!("{}:{}={source}", field.name, field.ty.source_text)
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "target={};kind={kind};fields=[{fields}]",
        fact.resolved_target.source_text
    )
}

#[test]
fn websocket_accept_reject_and_recursive_nominal_facts_snapshot() {
    let built = build(
        r#"
          import std

          type Context {
            roomId: string,
            cursor: string?,
          }

          function acceptFull() -> std.websocket.WebSocketConnectResult<Context> {
            return { tag: "accept", context: { roomId: "room", cursor: null }, businessIdentity: "biz", connectionPolicy: { maxConnections: 8, overflow: "close-oldest" } }
          }

          function acceptDefaults() -> std.websocket.WebSocketConnectResult<Context> {
            return { tag: "accept", context: { roomId: "room" } }
          }

          function reject() -> std.websocket.WebSocketConnectResult<Context> {
            return { tag: "reject", code: 403, reason: "denied" }
          }
        "#,
    )
    .expect("accept and reject object literals should type-check");

    let full = built.materialization(
        r#"{ tag: "accept", context: { roomId: "room", cursor: null }, businessIdentity: "biz", connectionPolicy: { maxConnections: 8, overflow: "close-oldest" } }"#,
    );
    assert_eq!(union_tag(full), Some("accept"));
    assert_eq!(
        field_names(full),
        ["businessIdentity", "connectionPolicy", "context", "tag"]
    );
    assert!(synthetic_fields(full).is_empty());

    let context = built.materialization(r#"{ roomId: "room", cursor: null }"#);
    assert!(matches!(
        context.kind,
        ObjectMaterializationKind::Record { .. }
    ));
    assert_eq!(field_names(context), ["cursor", "roomId"]);

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

    let defaults = built.materialization(r#"{ tag: "accept", context: { roomId: "room" } }"#);
    assert_eq!(union_tag(defaults), Some("accept"));
    assert_eq!(
        synthetic_fields(defaults),
        ["businessIdentity", "connectionPolicy"]
    );
    assert_eq!(
        fact_snapshot(defaults),
        "target=std.websocket.WebSocketConnectResult<Context>;kind=union(accept);fields=[businessIdentity:string?=synthetic-null,connectionPolicy:std.websocket.WebSocketConnectionPolicy?=synthetic-null,context:#0=provided,tag:\"accept\"=provided]"
    );
    let default_context = built.materialization(r#"{ roomId: "room" }"#);
    assert_eq!(synthetic_fields(default_context), ["cursor"]);
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
}

#[test]
fn object_literal_negatives_fail_closed() {
    let cases = [
        (
            r#"
              function targetless() -> void {
                const value = { label: "missing-target" }
              }
            "#,
            "requires an explicit target type",
        ),
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
