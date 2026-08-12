use std::sync::mpsc;

use serde_json::Value;

use crate::HttpNameValue;

/// One event from a real HTTP response stream.
#[derive(Debug)]
pub enum BytecodeHttpStreamEvent {
    Chunk(Vec<u8>),
    End,
    Error(String),
}

/// A live HTTP response stream owned by the host executor.
pub struct BytecodeHttpStream {
    pub status: u16,
    pub headers: Vec<HttpNameValue>,
    pub events: mpsc::Receiver<BytecodeHttpStreamEvent>,
    pub cancel: Box<dyn Fn() + Send + Sync>,
}

/// Host-owned HTTP effect executor exposed to the bytecode request adapter.
///
/// The request crate intentionally does not depend on `skiff-runtime-host`.
/// RuntimeHost constructs a concrete implementation and passes it through
/// `BytecodeRequestExecutionHandles`.
pub trait BytecodeHttpExecutor: Send + Sync + 'static {
    fn request(
        &self,
        input: Value,
        use_test_effects: bool,
        allow_unsafe_targets: bool,
    ) -> Result<Value, String>;

    fn stream(
        &self,
        input: Value,
        use_test_effects: bool,
        allow_unsafe_targets: bool,
    ) -> Result<BytecodeHttpStream, String>;
}

/// Exact deployment and test-case correlation facts needed for self-ingress.
#[derive(Clone, Debug, Default)]
pub struct BytecodeSelfIngressContext {
    pub origin: String,
    pub service_id: String,
    pub contract_version: String,
    pub test_case_capability: Option<String>,
    pub test_case_parent_request_id: Option<String>,
}
