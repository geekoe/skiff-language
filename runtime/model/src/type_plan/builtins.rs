//! Shared runtime builtin type catalog.
//!
//! Single source of truth for runtime builtin shapes (`string`, `Json`,
//! `Array`, `std.http.*`, `Db*Result`, `Duration`, ...), the pure name-parsing
//! helpers used by plan builders, and the artifact label helpers. Plan
//! builders in `skiff-runtime-linked-type-plan` and `skiff-runtime-boundary`
//! consume this catalog instead of maintaining private copies.

use super::{RuntimeRecordFieldPlan, RuntimeTypeIdentityPlan, RuntimeTypeNode, RuntimeTypePlan};
use skiff_artifact_model::TypeRefIr;

// ---- name parsing (moved from skiff-runtime-boundary::type_descriptor) ----

pub fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut angle_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ch if ch == delimiter && angle_depth == 0 && brace_depth == 0 && paren_depth == 0 => {
                let part = input[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    let part = input[start..].trim();
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

pub fn generic_text_parts(value: &str) -> Option<(&str, Vec<&str>)> {
    let value = value.trim();
    let start = value.find('<')?;
    if !value.ends_with('>') {
        return None;
    }
    let root = value[..start].trim();
    let inner = &value[start + 1..value.len() - 1];
    Some((root, split_top_level(inner, ',')))
}

pub fn generic_root(value: &str) -> Option<&str> {
    generic_text_parts(value)
        .map(|(root, _)| root)
        .or_else(|| Some(value.trim()).filter(|value| !value.is_empty()))
}

pub fn type_name_root(name: &str) -> &str {
    let name = name.trim();
    generic_text_parts(name)
        .map(|(root, _)| root)
        .unwrap_or_else(|| {
            name.find('<')
                .map(|index| name[..index].trim())
                .unwrap_or(name)
        })
}

pub fn bare_type_name(name: &str) -> &str {
    let root = type_name_root(name);
    let name = root
        .rsplit_once("::")
        .map(|(_, short)| short)
        .unwrap_or(root);
    name.rsplit(['.', ':']).next().unwrap_or(name).trim()
}

// ---- shape catalog ----

/// Canonical runtime builtin shape classification. Record-shaped builtins
/// (`std.http.*`, `Duration`) live in the record functions below and are not
/// enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeBuiltinShape {
    Array,
    Stream,
    Map,
    Json,
    JsonObject,
    Date,
    String,
    TaskRef,
    Integer,
    Number,
    Bool,
    Bytes,
    Null,
    DbInsertManyResult,
    DbUpdateManyResult,
    DbDeleteManyResult,
    DbUpsertResult,
}

impl RuntimeBuiltinShape {
    /// Resolves a builtin type name to its shape. Uses `bare_type_name` so
    /// full spellings (`std.collection.Array`, `std.http.Json`, ...) and the
    /// `bool`/`boolean` alias resolve like the historical leaf matches.
    pub fn of_name(name: &str) -> Option<Self> {
        Some(match bare_type_name(name) {
            "Array" => Self::Array,
            "Stream" => Self::Stream,
            "Map" => Self::Map,
            "Json" => Self::Json,
            "JsonObject" => Self::JsonObject,
            "Date" => Self::Date,
            "string" => Self::String,
            "TaskRef" => Self::TaskRef,
            "integer" => Self::Integer,
            "number" => Self::Number,
            "bool" | "boolean" => Self::Bool,
            "bytes" => Self::Bytes,
            "null" | "void" => Self::Null,
            "DbInsertManyResult" => Self::DbInsertManyResult,
            "DbUpdateManyResult" => Self::DbUpdateManyResult,
            "DbDeleteManyResult" => Self::DbDeleteManyResult,
            "DbUpsertResult" => Self::DbUpsertResult,
            _ => return None,
        })
    }

    /// Leaf node mapping; returns `None` for structural/record shapes.
    pub fn leaf_node(self) -> Option<RuntimeTypeNode> {
        Some(match self {
            Self::Json => RuntimeTypeNode::Json,
            Self::JsonObject => RuntimeTypeNode::JsonObject,
            Self::Date => RuntimeTypeNode::Date,
            Self::String => RuntimeTypeNode::String,
            Self::TaskRef => RuntimeTypeNode::TaskRef,
            Self::Integer => RuntimeTypeNode::Integer,
            Self::Number => RuntimeTypeNode::Number,
            Self::Bool => RuntimeTypeNode::Bool,
            Self::Bytes => RuntimeTypeNode::Bytes,
            Self::Null => RuntimeTypeNode::Null,
            _ => return None,
        })
    }
}

// ---- plan helpers ----

pub fn builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "builtin".to_string(),
        named_type_name: Some(name.to_string()),
        identity: RuntimeTypeIdentityPlan::default(),
        node,
    }
}

pub fn leaf_builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    builtin_plan(name, node)
}

pub fn std_field(name: &str, ty: RuntimeTypePlan) -> RuntimeRecordFieldPlan {
    let required = !matches!(ty.node, RuntimeTypeNode::Nullable(_));
    RuntimeRecordFieldPlan {
        name: name.to_string(),
        ty,
        required,
        identity: None,
    }
}

pub fn leaf_string_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("string", RuntimeTypeNode::String)
}

pub fn leaf_integer_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("integer", RuntimeTypeNode::Integer)
}

pub fn leaf_bytes_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("bytes", RuntimeTypeNode::Bytes)
}

pub fn std_record_plan(name: &str, fields: Vec<RuntimeRecordFieldPlan>) -> RuntimeTypePlan {
    builtin_plan(
        name,
        RuntimeTypeNode::Record {
            fields,
            boundary_record_kind: Some(name.to_string()),
        },
    )
}

pub fn std_nullable_plan(inner: RuntimeTypePlan) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "nullable".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::Nullable(Box::new(inner)),
    }
}

pub fn std_array_plan(item: RuntimeTypePlan) -> RuntimeTypePlan {
    builtin_plan("Array", RuntimeTypeNode::Array(Box::new(item)))
}

pub fn std_stream_plan(item: RuntimeTypePlan) -> RuntimeTypePlan {
    builtin_plan("Stream", RuntimeTypeNode::Stream(Box::new(item)))
}

pub fn std_http_header_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpHeader",
        vec![
            std_field("name", leaf_string_plan()),
            std_field("value", leaf_string_plan()),
        ],
    )
}

pub fn std_http_client_request_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientRequest",
        vec![
            std_field("method", leaf_string_plan()),
            std_field("url", leaf_string_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", std_nullable_plan(leaf_bytes_plan())),
            std_field("timeoutMs", std_nullable_plan(leaf_integer_plan())),
        ],
    )
}

pub fn std_http_client_response_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientResponse",
        vec![
            std_field("status", leaf_integer_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", leaf_bytes_plan()),
        ],
    )
}

pub fn std_http_client_stream_handle_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientStreamHandle",
        vec![
            std_field("status", leaf_integer_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", std_stream_plan(leaf_bytes_plan())),
        ],
    )
}

/// Inner node for the `std.http.*` builtin records; matches only the exact
/// `std.http.*` root spellings with zero args.
pub fn std_http_record_node(name: &str, arg_count: usize) -> Option<RuntimeTypeNode> {
    let root = type_name_root(name);
    let bare = bare_type_name(root);
    let node = match bare {
        "HttpClientRequest" if arg_count == 0 && root == "std.http.HttpClientRequest" => {
            std_http_client_request_plan().node
        }
        "HttpClientResponse" if arg_count == 0 && root == "std.http.HttpClientResponse" => {
            std_http_client_response_plan().node
        }
        "HttpClientStreamHandle" if arg_count == 0 && root == "std.http.HttpClientStreamHandle" => {
            std_http_client_stream_handle_plan().node
        }
        _ => return None,
    };
    Some(node)
}

pub fn std_duration_plan() -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "representation".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::Representation {
            type_name: "std.time.Duration".to_string(),
            payload: Box::new(leaf_integer_plan()),
        },
    }
}

// ---- Db*Result record templates ----

/// Fixed-shape Db*Result records (no recursive fields).
pub fn db_result_record_node(name: &str) -> Option<RuntimeTypeNode> {
    let root = bare_type_name(name);
    let fields = match root {
        "DbInsertManyResult" => vec![std_field(
            "insertedCount",
            leaf_builtin_plan("number", RuntimeTypeNode::Number),
        )],
        "DbUpdateManyResult" => vec![
            std_field(
                "matchedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            ),
            std_field(
                "modifiedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            ),
        ],
        "DbDeleteManyResult" => vec![std_field(
            "deletedCount",
            leaf_builtin_plan("number", RuntimeTypeNode::Number),
        )],
        _ => return None,
    };
    Some(RuntimeTypeNode::Record {
        fields,
        boundary_record_kind: Some(root.to_string()),
    })
}

/// `DbUpsertResult` record with a caller-supplied value plan (the recursion
/// semantics differ per input form and stay in the plan builders).
pub fn db_result_upsert_record_node(value: RuntimeTypePlan) -> RuntimeTypeNode {
    RuntimeTypeNode::Record {
        fields: vec![
            RuntimeRecordFieldPlan {
                name: "value".to_string(),
                ty: value,
                required: true,
                identity: None,
            },
            std_field("inserted", leaf_builtin_plan("bool", RuntimeTypeNode::Bool)),
        ],
        boundary_record_kind: Some("DbUpsertResult".to_string()),
    }
}

// ---- name-set predicates (moved from skiff-runtime-boundary) ----

pub fn is_builtin_named_type(name: &str) -> bool {
    RuntimeBuiltinShape::of_name(name).is_some_and(|shape| shape != RuntimeBuiltinShape::Stream)
}

pub fn is_builtin_concrete_type_name(name: &str) -> bool {
    matches!(
        name.trim(),
        "Json"
            | "JsonObject"
            | "Date"
            | "Stream"
            | "Config"
            | "DbInsertManyResult"
            | "DbUpdateManyResult"
            | "DbDeleteManyResult"
            | "DbUpsertResult"
    )
}

// ---- artifact label helpers ----

pub fn artifact_type_ref_label(type_ref: &TypeRefIr) -> &'static str {
    match type_ref {
        TypeRefIr::Builtin { .. } => "builtin",
        TypeRefIr::LocalType { .. } => "localType",
        TypeRefIr::PublicationType { .. } => "publicationType",
        TypeRefIr::ServiceSymbol { .. } => "serviceSymbol",
        TypeRefIr::PackageSymbol { .. } => "packageSymbol",
        TypeRefIr::PackageSchema { .. } => "packageSchema",
        TypeRefIr::AppliedNominal { .. } => "appliedNominal",
        TypeRefIr::DbObjectSymbol { .. } => "dbObjectSymbol",
        TypeRefIr::Record { .. } => "record",
        TypeRefIr::Union { .. } => "union",
        TypeRefIr::Nullable { .. } => "nullable",
        TypeRefIr::Literal { .. } => "literal",
        TypeRefIr::TypeParam { .. } => "typeParam",
        TypeRefIr::AnyInterface { .. } => "anyInterface",
        TypeRefIr::Function { .. } => "function",
    }
}

pub fn artifact_type_ref_named_type_name(type_ref: &TypeRefIr) -> Option<String> {
    match type_ref {
        TypeRefIr::Builtin { name, .. } => Some(name.clone()),
        _ => None,
    }
}
