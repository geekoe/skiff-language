use super::*;

/// Canonical runtime builtin shape classification. Layer 1 of the builtin
/// catalog: name → shape + leaf node mapping. Record-shaped builtins
/// (`std.http.*`, `Duration`) stay in the record functions below and are not
/// enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeBuiltinShape {
    Array,
    Stream,
    Map,
    Json,
    JsonObject,
    Date,
    String,
    Integer,
    Number,
    Bool,
    Bytes,
    Null,
    Void,
    DbInsertManyResult,
    DbUpdateManyResult,
    DbDeleteManyResult,
    DbUpsertResult,
}

impl RuntimeBuiltinShape {
    /// Resolves a builtin type name to its shape. Uses `bare_type_name` so
    /// full spellings (`std.collection.Array`, `std.http.Json`, ...) and the
    /// `bool`/`boolean` alias resolve exactly like the historical leaf matches.
    pub(crate) fn of_name(name: &str) -> Option<Self> {
        Some(match bare_type_name(name) {
            "Array" => Self::Array,
            "Stream" => Self::Stream,
            "Map" => Self::Map,
            "Json" => Self::Json,
            "JsonObject" => Self::JsonObject,
            "Date" => Self::Date,
            "string" => Self::String,
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
    pub(crate) fn leaf_node(self) -> Option<RuntimeTypeNode> {
        Some(match self {
            Self::Json => RuntimeTypeNode::Json,
            Self::JsonObject => RuntimeTypeNode::JsonObject,
            Self::Date => RuntimeTypeNode::Date,
            Self::String => RuntimeTypeNode::String,
            Self::Integer => RuntimeTypeNode::Integer,
            Self::Number => RuntimeTypeNode::Number,
            Self::Bool => RuntimeTypeNode::Bool,
            Self::Bytes => RuntimeTypeNode::Bytes,
            Self::Null | Self::Void => RuntimeTypeNode::Null,
            _ => return None,
        })
    }
}

fn builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "builtin".to_string(),
        named_type_name: Some(name.to_string()),
        identity: RuntimeTypeIdentityPlan::default(),
        node,
    }
}

/// Inner plan for a synthesized leaf builtin (string/integer/bytes/Json).
fn leaf_builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    builtin_plan(name, node)
}

fn std_field(name: &str, ty: RuntimeTypePlan) -> RuntimeRecordFieldPlan {
    let required = !matches!(ty.node, RuntimeTypeNode::Nullable(_));
    RuntimeRecordFieldPlan {
        name: name.to_string(),
        ty,
        required,
        identity: None,
    }
}

fn leaf_string_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("string", RuntimeTypeNode::String)
}

fn leaf_integer_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("integer", RuntimeTypeNode::Integer)
}

fn leaf_bytes_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("bytes", RuntimeTypeNode::Bytes)
}

fn std_record_plan(name: &str, fields: Vec<RuntimeRecordFieldPlan>) -> RuntimeTypePlan {
    builtin_plan(
        name,
        RuntimeTypeNode::Record {
            fields,
            boundary_record_kind: Some(name.to_string()),
        },
    )
}

fn std_nullable_plan(inner: RuntimeTypePlan) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "nullable".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::Nullable(Box::new(inner)),
    }
}

fn std_array_plan(item: RuntimeTypePlan) -> RuntimeTypePlan {
    builtin_plan("Array", RuntimeTypeNode::Array(Box::new(item)))
}

fn std_stream_plan(item: RuntimeTypePlan) -> RuntimeTypePlan {
    builtin_plan("Stream", RuntimeTypeNode::Stream(Box::new(item)))
}

fn std_http_header_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpHeader",
        vec![
            std_field("name", leaf_string_plan()),
            std_field("value", leaf_string_plan()),
        ],
    )
}

fn std_http_client_request_plan() -> RuntimeTypePlan {
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

fn std_http_client_response_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientResponse",
        vec![
            std_field("status", leaf_integer_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", leaf_bytes_plan()),
        ],
    )
}

fn std_http_client_stream_handle_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientStreamHandle",
        vec![
            std_field("status", leaf_integer_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", std_stream_plan(leaf_bytes_plan())),
        ],
    )
}

pub(crate) fn std_runtime_builtin_node(
    name: &str,
    arg_count: usize,
) -> Option<Result<RuntimeTypeNode>> {
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
    Some(Ok(node))
}

pub(crate) fn native_builtin_plan(name: &str) -> Result<RuntimeTypePlan> {
    if name == "Duration" || name == "std.time.Duration" {
        return Ok(RuntimeTypePlan {
            label: "representation".to_string(),
            named_type_name: None,
            identity: RuntimeTypeIdentityPlan::default(),
            node: RuntimeTypeNode::Representation {
                type_name: "std.time.Duration".to_string(),
                payload: Box::new(leaf_integer_plan()),
            },
        });
    }
    if let Some(node) = std_runtime_builtin_node(name, 0) {
        return Ok(builtin_plan(name, node?));
    }
    let node = RuntimeBuiltinShape::of_name(name)
        .and_then(RuntimeBuiltinShape::leaf_node)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "native signature references unknown builtin type {name}"
            ))
        })?;
    Ok(builtin_plan(name, node))
}

pub(crate) fn db_result_node_from_parts(
    root: &str,
    args: &[skiff_artifact_model::TypeRefIr],
) -> Option<Result<RuntimeTypeNode>> {
    let node = match bare_type_name(root) {
        "DbInsertManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "insertedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpdateManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![
                std_field(
                    "matchedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
                std_field(
                    "modifiedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
            ],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbDeleteManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "deletedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpsertResult" if args.len() == 1 => {
            return Some(
                RuntimeTypePlan::from_artifact_type_ref(&args[0]).map(|value| {
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
                        boundary_record_kind: Some(root.to_string()),
                    }
                }),
            );
        }
        _ => return None,
    };
    Some(Ok(node))
}

pub(crate) fn db_result_node_from_artifact_parts_in_program(
    root: &str,
    args: &[skiff_artifact_model::TypeRefIr],
    ctx: &PlanContext<'_>,
) -> Option<Result<RuntimeTypeNode>> {
    let node = match bare_type_name(root) {
        "DbInsertManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "insertedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpdateManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![
                std_field(
                    "matchedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
                std_field(
                    "modifiedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
            ],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbDeleteManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "deletedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpsertResult" if args.len() == 1 => {
            return Some(
                RuntimeTypePlan::from_artifact_type_ref_in_program_ref(&args[0], ctx).map(
                    |value| RuntimeTypeNode::Record {
                        fields: vec![
                            RuntimeRecordFieldPlan {
                                name: "value".to_string(),
                                ty: value,
                                required: true,
                                identity: None,
                            },
                            std_field("inserted", leaf_builtin_plan("bool", RuntimeTypeNode::Bool)),
                        ],
                        boundary_record_kind: Some(root.to_string()),
                    },
                ),
            );
        }
        _ => return None,
    };
    Some(Ok(node))
}

pub(crate) fn db_result_node_from_linked_parts(
    root: &str,
    args: &[LinkedTypeRef],
    ctx: &PlanContext<'_>,
) -> Option<Result<RuntimeTypeNode>> {
    let node = match bare_type_name(root) {
        "DbInsertManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "insertedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpdateManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![
                std_field(
                    "matchedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
                std_field(
                    "modifiedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
            ],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbDeleteManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "deletedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpsertResult" if args.len() == 1 => {
            return Some(
                RuntimeTypePlan::from_linked_ref(&args[0], &ctx.deeper_by(2)).map(|value| {
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
                        boundary_record_kind: Some(root.to_string()),
                    }
                }),
            );
        }
        _ => return None,
    };
    Some(Ok(node))
}
