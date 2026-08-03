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

/// Normalized input view over the three builtin entry forms. Depth accounting
/// is owned by each variant so the historical per-input semantics stay intact:
/// `Artifact` never deepens, `ArtifactInProgram` keeps the caller ctx without
/// deepening, and `Linked` deepens by 2 (the JSON `args`-array nesting the
/// reference walk encodes for builtin arguments).
pub(crate) enum PlanInput<'a> {
    Artifact {
        name: &'a str,
        args: &'a [skiff_artifact_model::TypeRefIr],
    },
    ArtifactInProgram {
        name: &'a str,
        args: &'a [skiff_artifact_model::TypeRefIr],
    },
    Linked {
        name: &'a str,
        args: &'a [LinkedTypeRef],
    },
}

impl<'a> PlanInput<'a> {
    pub(crate) fn bare_name(&self) -> &str {
        match self {
            Self::Artifact { name, .. }
            | Self::ArtifactInProgram { name, .. }
            | Self::Linked { name, .. } => bare_type_name(name),
        }
    }

    fn arg_count(&self) -> usize {
        match self {
            Self::Artifact { args, .. } => args.len(),
            Self::ArtifactInProgram { args, .. } => args.len(),
            Self::Linked { args, .. } => args.len(),
        }
    }

    fn is_array(&self) -> bool {
        match self {
            // The linked entry historically matched only the exact `Array`
            // spelling; the artifact entries matched through `bare_type_name`.
            Self::Linked { name, .. } => *name == "Array",
            _ => self.bare_name() == "Array",
        }
    }

    fn is_map(&self) -> bool {
        match self {
            Self::Linked { name, .. } => *name == "Map",
            _ => self.bare_name() == "Map",
        }
    }

    fn is_stream(&self) -> bool {
        self.bare_name() == "Stream"
    }

    pub(crate) fn recurse_arg_plan(
        &self,
        index: usize,
        ctx: Option<&PlanContext<'_>>,
    ) -> Result<RuntimeTypePlan> {
        match self {
            Self::Artifact { args, .. } => RuntimeTypePlan::from_artifact_type_ref(&args[index]),
            Self::ArtifactInProgram { args, .. } => {
                RuntimeTypePlan::from_artifact_type_ref_in_program_ref(
                    &args[index],
                    ctx.expect("artifact-in-program input requires a plan context"),
                )
            }
            Self::Linked { args, .. } => RuntimeTypePlan::from_linked_ref(
                &args[index],
                &ctx.expect("linked input requires a plan context")
                    .deeper_by(2),
            ),
        }
    }
}

/// Structural Array/Map/Stream branches shared by the three builtin entries.
pub(crate) fn structural_builtin_node(
    input: &PlanInput<'_>,
    ctx: Option<&PlanContext<'_>>,
) -> Option<Result<RuntimeTypeNode>> {
    let count = input.arg_count();
    if input.is_array() && count == 1 {
        return Some(
            input
                .recurse_arg_plan(0, ctx)
                .map(|plan| RuntimeTypeNode::Array(Box::new(plan))),
        );
    }
    if input.is_map() && count == 2 {
        return Some(input.recurse_arg_plan(0, ctx).and_then(|key| {
            input
                .recurse_arg_plan(1, ctx)
                .map(|value| RuntimeTypeNode::Map {
                    key: Box::new(key),
                    value: Box::new(value),
                })
        }));
    }
    if input.is_stream() && count == 1 {
        return Some(
            input
                .recurse_arg_plan(0, ctx)
                .map(|plan| RuntimeTypeNode::Stream(Box::new(plan))),
        );
    }
    None
}

/// Single Db*Result catalog shared by the three builtin entries. The only
/// per-input difference is `DbUpsertResult`'s value recursion, which lives in
/// [`PlanInput::recurse_arg_plan`].
pub(crate) fn db_result_node(
    input: &PlanInput<'_>,
    ctx: Option<&PlanContext<'_>>,
) -> Option<Result<RuntimeTypeNode>> {
    let root = input.bare_name();
    let count = input.arg_count();
    let node = match root {
        "DbInsertManyResult" if count == 0 => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "insertedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpdateManyResult" if count == 0 => RuntimeTypeNode::Record {
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
        "DbDeleteManyResult" if count == 0 => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "deletedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpsertResult" if count == 1 => {
            return Some(
                input
                    .recurse_arg_plan(0, ctx)
                    .map(|value| RuntimeTypeNode::Record {
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
                    }),
            );
        }
        _ => return None,
    };
    Some(Ok(node))
}
