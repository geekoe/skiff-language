use std::{collections::HashMap, error::Error, fmt, sync::Mutex as StdMutex};

use skiff_runtime_model::error::{RuntimeErrorPayload, WirePayload};
use tokio::sync::mpsc;

use crate::{HttpResponseMetadata, ResponseError};

#[derive(Debug, Clone, PartialEq)]
pub enum OutboundResponse {
    Start { http_response: HttpResponseMetadata },
    Chunk { seq: u64, payload: Vec<u8> },
    End { payload: Vec<u8> },
    Error(ResponseError),
}

impl OutboundResponse {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Start { .. } => "response.start",
            Self::Chunk { .. } => "response.chunk",
            Self::End { .. } => "response.end",
            Self::Error(_) => "response.error",
        }
    }
}

pub type OutboundResponseReceiver = mpsc::UnboundedReceiver<OutboundResponse>;
pub type OutboundResponseSender = mpsc::UnboundedSender<OutboundResponse>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundRequestRegistryError {
    LockPoisoned,
    DuplicateRequestId(String),
}

impl fmt::Display for OutboundRequestRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LockPoisoned => formatter.write_str("outbound request registry lock is poisoned"),
            Self::DuplicateRequestId(request_id) => {
                write!(formatter, "duplicate outbound request id {request_id}")
            }
        }
    }
}

impl Error for OutboundRequestRegistryError {}

impl WirePayload for OutboundRequestRegistryError {
    fn payload(&self) -> RuntimeErrorPayload {
        RuntimeErrorPayload {
            code: "InternalError".to_string(),
            message: self.to_string(),
            status: None,
            details: None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Default)]
pub struct OutboundRequestRegistry {
    pending: StdMutex<HashMap<String, OutboundResponseSender>>,
}

impl OutboundRequestRegistry {
    pub fn insert(
        &self,
        request_id: String,
        sender: OutboundResponseSender,
    ) -> Result<(), OutboundRequestRegistryError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| OutboundRequestRegistryError::LockPoisoned)?;
        if pending.insert(request_id.clone(), sender).is_some() {
            return Err(OutboundRequestRegistryError::DuplicateRequestId(request_id));
        }
        Ok(())
    }

    pub fn complete(&self, request_id: &str) -> Option<OutboundResponseSender> {
        self.pending.lock().ok()?.remove(request_id)
    }

    pub fn sender(&self, request_id: &str) -> Option<OutboundResponseSender> {
        self.pending.lock().ok()?.get(request_id).cloned()
    }

    pub fn contains(&self, request_id: &str) -> bool {
        self.pending
            .lock()
            .is_ok_and(|pending| pending.contains_key(request_id))
    }

    pub fn contains_matching(&self, mut matches: impl FnMut(&str) -> bool) -> bool {
        self.pending
            .lock()
            .is_ok_and(|pending| pending.keys().any(|request_id| matches(request_id)))
    }

    pub fn remove(&self, request_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(request_id);
        }
    }
}
