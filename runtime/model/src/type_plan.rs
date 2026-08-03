use std::fmt;

use crate::service_error::CatchIdentity;

pub mod builtins;

pub use builtins::{
    artifact_type_ref_label, artifact_type_ref_named_type_name, bare_type_name, builtin_plan,
    db_result_record_node, db_result_upsert_record_node, generic_root, generic_text_parts,
    is_builtin_concrete_type_name, is_builtin_named_type, leaf_builtin_plan, leaf_bytes_plan,
    leaf_integer_plan, leaf_string_plan, split_top_level, std_array_plan, std_duration_plan,
    std_field, std_http_client_request_plan, std_http_client_response_plan,
    std_http_client_stream_handle_plan, std_http_header_plan, std_http_record_node,
    std_nullable_plan, std_record_plan, std_stream_plan, type_name_root, RuntimeBuiltinShape,
};

#[derive(Clone)]
pub struct RuntimeRecordFieldPlan {
    pub name: String,
    pub ty: RuntimeTypePlan,
    pub required: bool,
    pub identity: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeTypeIdentityPlan {
    pub catch_identity: Option<CatchIdentity>,
    pub interface: Option<String>,
    pub method_projection: Option<String>,
}

#[derive(Clone)]
pub struct RuntimeTypePlan {
    /// Display label for the type, captured at construction so error messages
    /// stay meaningful without retaining the full JSON descriptor.
    pub label: String,
    /// Named type name (e.g. a nominal alias) when the descriptor carries one,
    /// used by Map-key resolution to recognise string representations.
    pub named_type_name: Option<String>,
    /// Explicit ABI/runtime identity metadata. This stays absent for shape-only
    /// descriptors; callers that require identity must check it.
    pub identity: RuntimeTypeIdentityPlan,
    pub node: RuntimeTypeNode,
}

#[derive(Clone, Debug)]
pub enum RuntimeTypeNode {
    Alias(Box<RuntimeTypePlan>),
    Nullable(Box<RuntimeTypePlan>),
    Union(Vec<RuntimeTypePlan>),
    LiteralString(String),
    Representation {
        type_name: String,
        payload: Box<RuntimeTypePlan>,
    },
    Json,
    JsonObject,
    Bytes,
    Date,
    String,
    Bool,
    Number,
    Integer,
    /// Opaque `std.task.TaskRef` handle. The runtime value is the canonical
    /// `skiff-task-v1:<owner>.<taskId>` string; recoverable boundaries validate
    /// the canonical form before accepting it (never a plain string fallback).
    TaskRef,
    Null,
    Stream(Box<RuntimeTypePlan>),
    Array(Box<RuntimeTypePlan>),
    Map {
        key: Box<RuntimeTypePlan>,
        value: Box<RuntimeTypePlan>,
    },
    Record {
        fields: Vec<RuntimeRecordFieldPlan>,
        boundary_record_kind: Option<String>,
    },
    Unknown,
}

impl RuntimeTypePlan {
    pub fn new(
        label: impl Into<String>,
        named_type_name: Option<String>,
        node: RuntimeTypeNode,
    ) -> Self {
        Self {
            label: label.into(),
            named_type_name,
            identity: RuntimeTypeIdentityPlan::default(),
            node,
        }
    }

    pub fn synthetic_array(item: RuntimeTypePlan) -> Self {
        synthetic_builtin_plan("Array", RuntimeTypeNode::Array(Box::new(item)))
    }

    pub fn synthetic_map(key: RuntimeTypePlan, value: RuntimeTypePlan) -> Self {
        synthetic_builtin_plan(
            "Map",
            RuntimeTypeNode::Map {
                key: Box::new(key),
                value: Box::new(value),
            },
        )
    }

    pub fn synthetic_nullable(inner: RuntimeTypePlan) -> Self {
        Self {
            label: "nullable".to_string(),
            named_type_name: None,
            identity: RuntimeTypeIdentityPlan::default(),
            node: RuntimeTypeNode::Nullable(Box::new(inner)),
        }
    }

    pub fn synthetic_stream(item: RuntimeTypePlan) -> Self {
        synthetic_builtin_plan("Stream", RuntimeTypeNode::Stream(Box::new(item)))
    }

    pub fn json_value_plan() -> Self {
        synthetic_builtin_plan("Json", RuntimeTypeNode::Json)
    }

    pub fn json_object_plan() -> Self {
        synthetic_builtin_plan("JsonObject", RuntimeTypeNode::JsonObject)
    }

    /// Build a top-level synthetic request-payload `Record` plan from pre-built
    /// field plans.
    pub fn synthetic_request_record(fields: Vec<RuntimeRecordFieldPlan>) -> Self {
        Self {
            label: "record".to_string(),
            named_type_name: None,
            identity: RuntimeTypeIdentityPlan::default(),
            node: RuntimeTypeNode::Record {
                fields,
                boundary_record_kind: None,
            },
        }
    }

    pub fn synthetic_named_builtin(
        name: &str,
        node: RuntimeTypeNode,
        _args: Vec<RuntimeTypePlan>,
    ) -> Self {
        synthetic_builtin_plan(name, node)
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn named_type_name(&self) -> Option<&str> {
        self.named_type_name.as_deref()
    }

    pub fn identity(&self) -> &RuntimeTypeIdentityPlan {
        &self.identity
    }

    pub fn has_identity(&self) -> bool {
        self.identity.has_any()
    }

    pub fn interface_identity(&self) -> Option<&str> {
        self.identity.interface.as_deref()
    }

    pub fn catch_identity(&self) -> Option<&CatchIdentity> {
        self.identity.catch_identity.as_ref()
    }

    pub fn method_projection_identity(&self) -> Option<&str> {
        self.identity.method_projection.as_deref()
    }

    pub fn node(&self) -> &RuntimeTypeNode {
        &self.node
    }

    pub fn boundary_record_kind(&self) -> Option<&str> {
        match &self.node {
            RuntimeTypeNode::Record {
                boundary_record_kind,
                ..
            } => boundary_record_kind.as_deref(),
            _ => None,
        }
    }
}

impl RuntimeRecordFieldPlan {
    pub fn new(name: impl Into<String>, ty: RuntimeTypePlan, required: bool) -> Self {
        Self {
            name: name.into(),
            ty,
            required,
            identity: None,
        }
    }

    pub fn with_identity(mut self, identity: Option<String>) -> Self {
        self.identity = identity;
        self
    }

    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }
}

impl RuntimeTypeIdentityPlan {
    pub fn has_any(&self) -> bool {
        self.catch_identity.is_some()
            || self.interface.is_some()
            || self.method_projection.is_some()
    }
}

impl fmt::Debug for RuntimeRecordFieldPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("RuntimeRecordFieldPlan");
        debug
            .field("name", &self.name)
            .field("ty", &self.ty)
            .field("required", &self.required);
        if self.identity.is_some() {
            debug.field("identity", &self.identity);
        }
        debug.finish()
    }
}

impl fmt::Debug for RuntimeTypePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("RuntimeTypePlan");
        debug
            .field("label", &self.label)
            .field("named_type_name", &self.named_type_name);
        if self.identity.has_any() {
            debug.field("identity", &self.identity);
        }
        debug.field("node", &self.node).finish()
    }
}

fn synthetic_builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "builtin".to_string(),
        named_type_name: Some(name.to_string()),
        identity: RuntimeTypeIdentityPlan::default(),
        node,
    }
}

#[cfg(test)]
mod tests;
