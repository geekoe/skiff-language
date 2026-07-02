use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_runtime_eval::EvalRuntimeProgram;
use skiff_runtime_linked_program::ExecutableAddr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOperationParameter {
    pub name: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOperation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_abi_id: Option<String>,
    pub operation: String,
    pub target: String,
    pub mode: String,
    #[serde(default)]
    pub parameters: Vec<RuntimeOperationParameter>,
    #[serde(default)]
    pub service_protocol_identity: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestServiceMetadata {
    pub service_id: String,
    pub service_version: String,
    pub runtime_id: String,
    pub build_id: String,
    pub http_response_max_bytes: usize,
}

#[derive(Clone)]
pub struct RequestOperationContext {
    pub metadata: RequestServiceMetadata,
    pub eval_program: Arc<EvalRuntimeProgram>,
    pub operation: RuntimeOperation,
    pub addr: ExecutableAddr,
}

impl RequestOperationContext {
    pub fn new(
        metadata: RequestServiceMetadata,
        eval_program: Arc<EvalRuntimeProgram>,
        operation: RuntimeOperation,
        addr: ExecutableAddr,
    ) -> Self {
        Self {
            metadata,
            eval_program,
            operation,
            addr,
        }
    }
}
