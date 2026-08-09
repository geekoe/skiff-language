use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileIrRef {
    pub file_ir_identity: String,
    pub module_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ast_hash: Option<String>,
}

impl FileIrRef {
    pub fn new(file_ir_identity: impl Into<String>, module_path: impl Into<String>) -> Self {
        Self {
            file_ir_identity: file_ir_identity.into(),
            module_path: module_path.into(),
            artifact_path: None,
            source_ast_hash: None,
        }
    }
}

/// Lightweight package-artifact reference to a canonical bytecode record.
/// The bytecode artifact itself lives in the same package build record family
/// and is content-addressed by `bytecode_identity` (C9 identity/content
/// consistency is validated by artifact-identity).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytecodeArtifactRef {
    pub bytecode_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
}

impl BytecodeArtifactRef {
    pub fn new(bytecode_identity: impl Into<String>) -> Self {
        Self {
            bytecode_identity: bytecode_identity.into(),
            artifact_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSpanRef {
    pub source_id: u64,
    pub start: SourcePosition,
    pub end: SourcePosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourcePosition {
    pub line: u32,
    pub column: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

impl SourcePosition {
    pub fn new(line: u32, column: u32) -> Self {
        Self {
            line,
            column,
            offset: None,
        }
    }
}
